//! `net` task: manage the connection to the external cluster network.
//!
//! - `net switch <bastion-ip> <dev|prod>` — regenerate `~/.ssh/config` for the
//!   bastion and switch the network/webserver/applications submodules to the
//!   matching branch (ported from `net-switch.sh`).
//! - `net bastion <host>` — open an ssh session (ported from `bastion.sh`).
//! - `net status` — show submodule status and the current ssh config
//!   (ported from `net-status.sh`).

use aws::ssm;
use clap::{Args, Subcommand};
use guss_common::error::Result;
use guss_common::{clusters, config, git, paths, ui};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;

mod template;

#[derive(Args)]
pub struct NetArgs {
    #[command(subcommand)]
    command: NetCmd,
}

#[derive(Subcommand)]
enum NetCmd {
    /// Initialize deployment mode (dev/prod/test) — required before using SSM commands.
    Init,
    /// Show or change the current deployment mode.
    Mode {
        /// New mode (dev/prod/test). Omit to show current mode.
        new_mode: Option<String>,
    },
    /// Open an SSM session or run a command on a service.
    Ssm {
        /// Service name (will be prefixed with current mode).
        service: String,
        /// Command to run non-interactively (opens session if omitted).
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Show network submodule status and the active ssh config.
    Status,
    /// Assemble vn-network-launch's staging includes into a deployable CFN template.
    Build {
        /// Master file to resolve (defaults to staging/ready-to-build.yml).
        #[arg(long)]
        master: Option<PathBuf>,
        /// Output file (defaults to staging/builds/new-build.yml).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Print the assembled template without writing it.
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn run(args: NetArgs) -> Result<()> {
    match args.command {
        NetCmd::Init => init(),
        NetCmd::Mode { new_mode } => mode_cmd(new_mode),
        NetCmd::Ssm { service, command } => ssm_cmd(&service, &command),
        NetCmd::Status => status(),
        NetCmd::Build { master, out, dry_run } => build_template(master, out, dry_run),
    }
}

fn collect_cluster_repos() -> Vec<String> {
    let root = match paths::project_guss() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let vapi_root = match paths::project_vapi() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let cluster_configs = match clusters::load() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut repos = HashSet::new();
    for cluster in cluster_configs {
        // Only collect repos from net (network/remote) clusters
        if cluster.environment != "net" {
            continue;
        }

        for (container_name, cfg) in &cluster.containers {
            // Skip if explicitly marked to not switch branches
            if cfg.no_branch_switch {
                continue;
            }

            // If repo is explicitly declared, use it
            if let Some(repo_rel) = &cfg.repo {
                repos.insert(root.join(repo_rel).to_string_lossy().to_string());
            } else if let Ok(resolved) = cluster.resolve(container_name, &root, &vapi_root) {
                // Otherwise, if the source itself is a git repo root, use it
                if git::is_repo(&resolved.source) {
                    repos.insert(resolved.source.to_string_lossy().to_string());
                }
            }
        }
    }
    repos.into_iter().collect()
}

fn init() -> Result<()> {
    let mut settings = config::Settings::load()?;

    ui::heading("net init");

    if settings.mode.is_some() {
        ui::warn(&format!("mode is already set to: {}", settings.mode.unwrap()));
        ui::info("Use 'guss net mode <dev|prod|test>' to change it.");
        return Ok(());
    }

    ui::info("Select your deployment mode (used to prefix service names for SSM):");
    ui::info("  dev   — development environment");
    ui::info("  prod  — production environment");
    ui::info("  test  — testing environment");

    print!("Enter mode (dev/prod/test): ");
    io::stdout().flush()?;
    let mut mode_str = String::new();
    io::stdin().read_line(&mut mode_str)?;
    let mode: config::Mode = mode_str.trim().parse().map_err(|e| guss_common::error::anyhow!("{}", e))?;

    settings.set_mode(mode)?;
    ui::good(&format!("mode set to: {}", mode));
    Ok(())
}

fn mode_cmd(new_mode: Option<String>) -> Result<()> {
    let mut settings = config::Settings::load()?;

    ui::heading("deployment mode");

    match new_mode {
        None => {
            let current = settings.get_mode()?;
            ui::info(&format!("current mode: {}", current));
        }
        Some(mode_str) => {
            let mode: config::Mode = mode_str.parse().map_err(|e| guss_common::error::anyhow!("{}", e))?;

            // Collect all repos to manage: tracked_repos + applications/webserver + cluster container repos
            let home = paths::home()?;
            let mut all_repos = settings.tracked_repos.clone();
            all_repos.push(settings.get_app_repo_path("applications")?.to_string_lossy().to_string());
            all_repos.push(settings.get_app_repo_path("webserver")?.to_string_lossy().to_string());
            all_repos.extend(collect_cluster_repos());
            // Dedupe while preserving order
            let mut seen = HashSet::new();
            all_repos.retain(|r| seen.insert(r.clone()));

            // Check for uncommitted changes before switching branches.
            // For test mode, only check repos on prod (they must switch off prod).
            // For dev/prod modes, check all repos (they must switch to specific branches).
            let mut dirty_repos = Vec::new();
            for repo_path in &all_repos {
                let abs_path = if repo_path.starts_with("~/") {
                    home.join(repo_path.trim_start_matches("~/"))
                } else {
                    std::path::PathBuf::from(repo_path)
                };

                if abs_path.exists() && git::is_repo(&abs_path) {
                    let should_check = if mode == config::Mode::Test {
                        // For test mode, only check repos currently on prod
                        git::current_branch(&abs_path).map(|b| b == "prod").unwrap_or(false)
                    } else {
                        // For dev/prod modes, check all repos
                        true
                    };

                    if should_check && !git::is_clean(&abs_path) {
                        dirty_repos.push(repo_path.clone());
                    }
                }
            }

            if !dirty_repos.is_empty() {
                ui::warn("Cannot switch branches: uncommitted changes in:");
                for repo in &dirty_repos {
                    println!("  - {}", repo);
                }
                return Err(guss_common::error::anyhow!("commit or stash changes before changing mode"));
            }

            settings.set_mode(mode)?;
            ui::good(&format!("mode changed to: {}", mode));

            // Switch branches in all repos, collecting errors to report all at once
            let mut switch_errors = Vec::new();

            for repo_path in all_repos {
                let abs_path = if repo_path.starts_with("~/") {
                    home.join(repo_path.trim_start_matches("~/"))
                } else {
                    std::path::PathBuf::from(&repo_path)
                };

                if abs_path.exists() && git::is_repo(&abs_path) {
                    let current = git::current_branch(&abs_path).unwrap_or_default();
                    let target_branch = match mode {
                        config::Mode::Dev => Some("dev"),
                        config::Mode::Prod => Some("prod"),
                        config::Mode::Test => {
                            // Only switch to dev if currently on prod
                            if current == "prod" { Some("dev") } else { None }
                        }
                    };

                    if let Some(target) = target_branch {
                        if current != target {
                            ui::info(&format!("switching {} to {}", repo_path, target));
                            if let Err(e) = git::run(&abs_path, &["checkout", target], false) {
                                switch_errors.push((repo_path.clone(), format!("checkout failed: {}", e)));
                                continue;
                            }
                        }
                        // Pull latest changes on the target branch
                        if let Err(e) = git::run(&abs_path, &["pull"], false) {
                            switch_errors.push((repo_path.clone(), format!("pull failed: {}", e)));
                        }
                        // Show commits that were pulled in
                        let pulled_commits = git::run_optional(&abs_path, &["log", "@{1}..HEAD", "--oneline"]);
                        if !pulled_commits.is_empty() {
                            println!("Commits pulled:");
                            println!("{}", pulled_commits);
                        }
                        // Show only the tracking branch that was pulled from
                        if let Ok(current) = git::current_branch(&abs_path) {
                            if !current.is_empty() {
                                println!("Branch pulled from: origin/{}", current);
                            }
                        }
                    }
                }
            }

            if !switch_errors.is_empty() {
                ui::warn("Failed to switch branches in these repos:");
                for (repo, err) in &switch_errors {
                    println!("  - {}: {}", repo, err);
                }
            }
        }
    }

    Ok(())
}

fn ssm_cmd(service: &str, command: &[String]) -> Result<()> {
    let settings = config::Settings::load()?;
    let mode = settings.get_mode()?;

    if command.is_empty() {
        ui::heading(&format!("ssm → {mode}-{service}"));
        ssm::connect(mode, service)
    } else {
        let cmd = command.join(" ");
        ui::heading(&format!("ssm run → {mode}-{service} : {cmd}"));
        let output = ssm::run_command(mode, service, &cmd)?;
        println!("{output}");
        Ok(())
    }
}

fn build_template(master: Option<PathBuf>, out: Option<PathBuf>, dry_run: bool) -> Result<()> {
    let launch = paths::network_launch_dir()?;
    let master = master.unwrap_or_else(|| launch.join("staging/ready-to-build.yml"));
    let out = out.unwrap_or_else(|| launch.join("staging/builds/new-build.yml"));

    ui::heading("net build");
    ui::info(&format!("master -> {}\n     out -> {}", master.display(), out.display()));

    template::build(&master, &out, dry_run)?;
    if !dry_run {
        ui::good(&format!("wrote {}", out.display()));
    }
    Ok(())
}

fn status() -> Result<()> {
    let settings = config::Settings::load()?;

    ui::heading("deployment mode");
    match settings.mode {
        Some(mode) => ui::good(&format!("current mode: {}", mode)),
        None => ui::warn("mode not configured — run 'guss net init'"),
    }

    ui::heading("network submodule STATUS");
    let net = paths::network_dir()?;
    if git::is_repo(&net) {
        println!("{}", git::run_optional(&net, &["status"]));
    } else {
        ui::warn(&format!("{} is not a git repo", net.display()));
    }

    Ok(())
}
