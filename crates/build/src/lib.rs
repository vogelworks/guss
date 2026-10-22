//! `build` task: build apps per configuration and publish them.
//! Config-driven: build configs specify apps, sources, and destinations.
//!
//! - `build <config>` — build all apps in the named config, publish, commit, and deploy.
//! - `build <config> <app>` — build and publish a single app, with no git or deploy step.

use clap::Args;
use guss_common::error::{Result, anyhow};
use guss_common::{config, git, paths, proc, ui, version};
use std::fs;
use std::path::Path;
use std::thread;

#[derive(Args)]
#[command(disable_version_flag = true)]
pub struct BuildArgs {
    /// Build config name (e.g., "webapp", "desktop")
    config: String,

    /// Optional app name; if absent, build all apps in the config
    app: Option<String>,

    /// Skip confirmation prompts (for automation)
    #[arg(short, long)]
    yes: bool,

    /// Print commands without executing them
    #[arg(long)]
    dry_run: bool,

    /// Show version history and deployments for this build config
    #[arg(long)]
    details: bool,
}

pub fn run(args: BuildArgs) -> Result<()> {
    let settings = config::Settings::load()?;
    let mode = settings.get_mode()?;
    let app_config = settings.get_app_config(&args.config)?;

    if args.details {
        return show_build_details(&app_config);
    }

    match args.app {
        Some(app_name) => build_one(&app_config, &app_name, mode, args.dry_run),
        None => build_all(&app_config, mode, args.yes, args.dry_run),
    }
}

fn build_all(config: &config::BuildConfig, mode: config::Mode, yes: bool, dry_run: bool) -> Result<()> {
    let root = paths::project_guss()?;
    let src_repo = root.join(&config.root_src);
    let dest_repo = root.join(&config.root_dest);

    git::require_mode_branch(&src_repo, mode)?;
    git::require_mode_branch(&dest_repo, mode)?;

    // Read version from applications/package.json
    let version_to_use = version::pkg_version(&src_repo)?;

    // Version gating only in prod mode
    if mode == config::Mode::Prod {
        let target = format!("build:{}/{}", &config.name, &config.root_dest);
        version::check_table_only(&root, &target, &version_to_use)?;
    }

    ui::heading("Ready to Build");
    println!("    mode -> {mode}\n version -> {}", version_to_use);
    ui::require_confirm(&format!("Build all apps in '{}'?", config.name), yes)?;

    // Build apps in parallel
    ui::heading("Building apps");
    let mut handles = vec![];
    for app in &config.apps {
        let app_name = app.name.clone();
        let npm_script = app.npm_script.clone();
        let src_repo_clone = src_repo.clone();

        let handle = thread::spawn(move || proc::run_in(&src_repo_clone, "npm", &["run", &npm_script], false));
        handles.push((app_name, handle));
    }

    // Wait for all builds to complete and collect errors
    let mut build_errors = vec![];
    for (app_name, handle) in handles {
        match handle.join() {
            Ok(Ok(())) => {
                ui::good(&format!("  ✓ {app_name}"));
            }
            Ok(Err(e)) => {
                build_errors.push((app_name.clone(), e));
                ui::warn(&format!("  ✗ {app_name}"));
            }
            Err(_) => {
                build_errors.push((app_name.clone(), anyhow!("thread panicked")));
                ui::warn(&format!("  ✗ {app_name} (thread panic)"));
            }
        }
    }

    if !build_errors.is_empty() {
        let errors = build_errors
            .iter()
            .map(|(name, err)| format!("  {name}: {err}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(anyhow!("Build failed:\n{errors}"));
    }

    for app in &config.apps {
        publish_app(app, &src_repo, &dest_repo, dry_run)?;
    }

    // Commit and push dest repo after files are published
    if git::is_repo(&dest_repo) {
        println!("{}", git::run_optional(&dest_repo, &["status", "--short", "--branch"]));
    }

    let msg = commit_message("Publish apps for", mode, &version_to_use);
    let branch = mode.to_string();
    commit_and_push(&dest_repo, &branch, &msg, dry_run)?;

    // Version management only in prod mode
    if mode == config::Mode::Prod {
        // Sync version to destination if configured
        if config.sync_version {
            version::set_pkg_version(&dest_repo, &version_to_use)?;
        }

        // Record version to VERSIONS.json table
        let target = format!("build:{}/{}", &config.name, &config.root_dest);
        let mode_str = mode.to_string();
        version::record(&root, &target, &version_to_use, &mode_str, dry_run)?;
    }

    // Deploy if configured
    if let Some(deploy_config) = &config.deploy {
        ui::require_confirm("Deploy now?", yes)?;
        match deploy_config {
            config::BuildDeploy::Web { cluster, container } => {
                deploy::run(deploy::DeployArgs {
                    cluster: cluster.clone(),
                    container: vec![container.clone()],
                    tools: None,
                    tail: None,
                    yes,
                    dry_run,
                    no_cache: false,
                    details: false,
                })?;
            }
            config::BuildDeploy::Desktop { command } => {
                proc::run_in(&dest_repo, "sh", &["-c", command], dry_run)?;
            }
        }
    }

    // For prod, create version branch from prod, commit, push, and return to prod
    if mode == config::Mode::Prod {
        tag_or_merge_version_branch(&dest_repo, &version_to_use, dry_run)?;
        proc::run_in(&dest_repo, "git", &["checkout", &branch], dry_run)?;
    }

    ui::good("build complete");
    Ok(())
}

fn build_one(config: &config::BuildConfig, app_name: &str, mode: config::Mode, dry_run: bool) -> Result<()> {
    let root = paths::project_guss()?;
    let src_repo = root.join(&config.root_src);
    let dest_repo = root.join(&config.root_dest);

    let app = config
        .apps
        .iter()
        .find(|a| a.name == app_name)
        .ok_or_else(|| anyhow!("app '{}' not found in config '{}'", app_name, config.name))?;

    ui::heading("Ready to Build");
    println!("     app -> {app_name}\n    mode -> {mode}");

    proc::run_in(&src_repo, "npm", &["run", &app.npm_script], dry_run)?;
    publish_app(app, &src_repo, &dest_repo, dry_run)?;

    ui::good(&format!("published {app_name} (no git, no deploy)"));
    Ok(())
}

fn commit_message(verb: &str, mode: config::Mode, version: &str) -> String {
    format!("{verb} {mode} - version {version}")
}

fn commit_and_push(dir: &Path, branch: &str, message: &str, dry_run: bool) -> Result<()> {
    proc::run_in(dir, "git", &["add", "."], dry_run)?;
    let _ = proc::run_in(dir, "git", &["commit", "-m", message], dry_run);
    proc::run_in(dir, "git", &["push", "origin", branch], dry_run)
}

/// Create a `v<version>` branch for prod releases, or merge into it if it exists.
fn tag_or_merge_version_branch(dir: &Path, version: &str, dry_run: bool) -> Result<()> {
    let branch = format!("v{version}");

    // Check if the version branch already exists
    if git::branch_exists(dir, &branch) {
        // Branch exists: merge current HEAD into it
        proc::run_in(dir, "git", &["checkout", &branch], dry_run)?;
        proc::run_in(dir, "git", &["merge", "--no-edit", "HEAD@{1}"], dry_run)?;
        proc::run_in(dir, "git", &["push", "origin", &branch], dry_run)
    } else {
        // Branch doesn't exist: create and push it
        proc::run_in(dir, "git", &["checkout", "-b", &branch], dry_run)?;
        proc::run_in(dir, "git", &["push", "-u", "origin", &branch], dry_run)
    }
}

/// Copy built app from src repo into dest repo.
fn publish_app(app: &config::BuildApp, src_repo: &Path, dest_repo: &Path, dry_run: bool) -> Result<()> {
    let src = src_repo.join(&app.src_dir);
    let dest = dest_repo.join(&app.dest_path);
    let dest_s = dest.to_string_lossy().to_string();

    if !dry_run {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
    }
    proc::run("sudo", &["rm", "-rf", &dest_s], dry_run)?;
    proc::run("cp", &["-r", &src.to_string_lossy(), &dest_s], dry_run)?;
    ui::info(&format!("  published {} -> {}", app.name, dest.display()));
    Ok(())
}

fn show_build_details(config: &config::BuildConfig) -> Result<()> {
    let root = paths::project_guss()?;
    let (versions, deployments) = version::get_build_details(&root, &config.name, &config.root_dest)?;

    if versions.is_empty() && deployments.is_empty() {
        ui::info("No version history found");
        return Ok(());
    }

    ui::heading(&format!("Build History: {}", config.name));

    if !versions.is_empty() {
        println!("\nVersions:");
        for (target, ver, date) in versions {
            println!("  {} v{} ({})", target, ver, date);
        }
    }

    if !deployments.is_empty() {
        println!("\nDeployments to {}:", config.root_dest);
        for (target, date, version, mode) in deployments {
            let version_str = version.map(|v| format!(" v{}", v)).unwrap_or_default();
            println!("  {}{} ({}@{})", target, version_str, mode, date);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_message_includes_version() {
        assert_eq!(commit_message("Build for", config::Mode::Dev, "1.0.0"), "Build for dev - version 1.0.0");
        assert_eq!(commit_message("Build for", config::Mode::Prod, "2.4.0"), "Build for prod - version 2.4.0");
    }
}
