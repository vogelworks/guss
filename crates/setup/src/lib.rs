//! `setup` task: install the toolset and manage this machine's environment.
//!
//! Replaces the old model (`guss-setup.sh` + `setups/*.sh` copying each `.sh`
//! into `/usr/local/bin`). The lifecycle is split by who runs it:
//!
//! * `init` configures a machine that already has a `guss` binary — the path a
//!   teammate with no Rust toolchain takes;
//! * `install` is the build-from-source path, for contributors;
//! * `publish`/`update` move signed release tarballs (see [`release`]);
//! * `uninstall` reverses `init`/`install`.
//!
//! Also home to the `login`/`status`/`logout` wrappers over `guss_common::auth`.

pub mod release;

use clap::{Args, Subcommand};
use guss_common::error::{Result, anyhow};
use guss_common::{config, paths, proc, ui};
use std::fs;
use std::path::{Path, PathBuf};

/// (release binary name, installed command name) for every workspace binary.
/// Every task (`net`, `build`, `deploy`, `vapi`, `stack`, `docs`, `jump`, `setup`,
/// `project`) is a subcommand of `guss`, not its own binary — `guss` is the
/// only binary this installs.
const BINARIES: &[(&str, &str)] = &[("guss", "guss")];

/// The submodule repos that `install`/`git-pull` refresh.
const REPOS: &[&str] = &["applications", "webserver", "network"];

/// Marker + body of the `jump` shell function appended to the user's rc.
const JUMP_FN_MARKER: &str = "# guss-jump";
const JUMP_FN_END: &str = "# guss-jump-end";
const JUMP_FN_BODY: &str = "jump() { local d; d=\"$(guss jump \"$@\")\" || return; cd \"$d\"; }\n";

/// Default install location. `~/.local/bin` needs no `sudo`, unlike the old
/// `/usr/local/bin`.
const DEFAULT_PREFIX: &str = "~/.local/bin";

#[derive(Args)]
pub struct SetupArgs {
    #[command(subcommand)]
    command: SetupCmd,
}

#[derive(Subcommand)]
enum SetupCmd {
    /// Configure this machine for an already-installed `guss` (no cargo needed).
    Init {
        /// The project-guss checkout to record. Default: detected from the cwd.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Directory the `guss` command lives in, recorded for uninstall.
        #[arg(long, default_value = DEFAULT_PREFIX)]
        prefix: String,
        /// Skip pulling the submodule repos.
        #[arg(long)]
        no_pull: bool,
        #[arg(short, long)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Build from source and install, then run the `init` steps.
    Install {
        /// Directory to install the binary into.
        #[arg(long, default_value = DEFAULT_PREFIX)]
        prefix: String,
        #[arg(short, long)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the installed binary and the shell function.
    Uninstall {
        #[arg(long, default_value = DEFAULT_PREFIX)]
        prefix: String,
        /// Also delete `~/.config/guss` (keys, tokens, stack registry).
        #[arg(long)]
        purge: bool,
        #[arg(short, long)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Generate this machine's release-signing key and print its public half.
    Keygen {
        /// Overwrite an existing key. The old public key stops verifying.
        #[arg(long)]
        force: bool,
        #[arg(short, long)]
        yes: bool,
    },
    /// Build, sign, and publish a release tarball to the central server.
    Publish {
        /// Channel to publish to.
        channel: release::Channel,
        #[arg(short, long)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Download and install a newer signed release.
    Update {
        /// Channel to fetch from.
        #[arg(long, default_value = "prod")]
        channel: release::Channel,
        /// Install this exact version, even if it is a downgrade.
        #[arg(long)]
        tag: Option<String>,
        #[arg(long, default_value = DEFAULT_PREFIX)]
        prefix: String,
        #[arg(short, long)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Pull the submodule repos (applications, webserver, network).
    GitPull {
        #[arg(long)]
        dry_run: bool,
    },
    /// Print the reference list of guss admin commands.
    Reference,
    /// Log in to the central login server and cache a session token.
    Login {
        /// Username to authenticate as.
        user: String,
    },
    /// Show the current login/session status.
    Status,
    /// Clear the cached session token.
    Logout,
}

impl SetupArgs {
    /// Permission key for the auth allow-list, or `None` if this subcommand
    /// must always be runnable regardless of login state (you need to be
    /// able to log in, check status, or bootstrap a machine even with no valid
    /// session — `init`/`update` are how a machine reaches a usable state).
    pub fn permission_key(&self) -> Option<String> {
        match &self.command {
            SetupCmd::Login { .. } | SetupCmd::Status | SetupCmd::Logout | SetupCmd::Init { .. } | SetupCmd::Update { .. } => None,
            SetupCmd::Publish { .. } => Some("setup:publish".to_string()),
            _ => Some("setup".to_string()),
        }
    }
}

pub fn run(args: SetupArgs) -> Result<()> {
    match args.command {
        SetupCmd::Init {
            root,
            prefix,
            no_pull,
            yes,
            dry_run,
        } => init(root.as_deref(), &prefix, no_pull, yes, dry_run),
        SetupCmd::Install { prefix, yes, dry_run } => install(&prefix, yes, dry_run),
        SetupCmd::Uninstall { prefix, purge, yes, dry_run } => uninstall(&prefix, purge, yes, dry_run),
        SetupCmd::Keygen { force, yes } => release::keygen(force, yes),
        SetupCmd::Publish { channel, yes, dry_run } => release::publish(channel, yes, dry_run),
        SetupCmd::Update {
            channel,
            tag,
            prefix,
            yes,
            dry_run,
        } => release::update(channel, tag.as_deref(), &expand_prefix(&prefix)?, yes, dry_run),
        SetupCmd::GitPull { dry_run } => git_pull(dry_run),
        SetupCmd::Reference => {
            print_help();
            Ok(())
        }
        SetupCmd::Login { user } => login_cmd(&user),
        SetupCmd::Status => status_cmd(),
        SetupCmd::Logout => logout_cmd(),
    }
}

/// Expand a leading `~/` in a prefix so `--prefix ~/.local/bin` works whether or
/// not the shell expanded it.
fn expand_prefix(prefix: &str) -> Result<PathBuf> {
    match prefix.strip_prefix("~/") {
        Some(rest) => Ok(paths::home()?.join(rest)),
        None => Ok(PathBuf::from(prefix)),
    }
}

// --- init -------------------------------------------------------------------

fn init(root: Option<&Path>, prefix: &str, no_pull: bool, yes: bool, dry_run: bool) -> Result<()> {
    let prefix = expand_prefix(prefix)?;
    let root = match root {
        Some(root) => root.to_path_buf(),
        None => detect_root()?,
    };

    ui::heading("guss setup init");
    println!(
        "  checkout  -> {}\n  config    -> {}\n  prefix    -> {}",
        root.display(),
        config::config_dir()?.display(),
        prefix.display()
    );
    ui::require_confirm("Write this machine's configuration?", yes)?;

    migrate_legacy_config(&root, dry_run)?;
    write_settings(&root, dry_run)?;
    ensure_mykey(dry_run)?;
    install_jump_function(dry_run)?;
    if !no_pull {
        git_pull(dry_run)?;
    }

    ui::good("init complete");
    ui::info("Reminders: put your aws credentials in ~/.aws, then run `guss net init` to set your deployment mode.");
    ui::info("Restart your shell (or `source ~/.bashrc`) to pick up the `jump` function.");
    Ok(())
}

/// Find the checkout by walking up from the cwd looking for `tools/Cargo.toml`,
/// falling back to whatever `paths` can resolve. Erroring here is better than
/// silently recording a wrong root.
fn detect_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    for dir in cwd.ancestors() {
        if dir.join("tools/Cargo.toml").exists() {
            return Ok(dir.to_path_buf());
        }
    }
    paths::project_guss().map_err(|_| anyhow!("could not find a project-guss checkout above {} — pass --root <dir>", cwd.display()))
}

/// Record the checkout in `~/.config/guss/config.toml`, preserving any `[auth]` /
/// `[release]` sections already there.
fn write_settings(root: &Path, dry_run: bool) -> Result<()> {
    let path = config::config_file()?;
    println!("  write {}", path.display());
    if dry_run {
        return Ok(());
    }
    let mut settings = config::Settings::load()?;
    settings.root = Some(root.to_path_buf());
    // The vapi services live in the checkout's `servers` submodule; only record
    // it when it actually exists, so the `~/project-vapi` default still applies
    // on machines laid out the old way.
    let servers = root.join("servers");
    if settings.vapi_root.is_none() && servers.is_dir() {
        settings.vapi_root = Some(servers);
    }
    settings.save()
}

/// Copy config out of the old in-repo `-config/` directory. Non-destructive: the
/// originals are left alone, and anything already migrated is not overwritten.
fn migrate_legacy_config(root: &Path, dry_run: bool) -> Result<()> {
    let legacy = root.join("-config");
    if !legacy.is_dir() {
        return Ok(());
    }
    let target = config::config_dir()?;
    let mut moved = Vec::new();
    for name in paths::CONFIG_FILES {
        let from = legacy.join(name);
        let to = target.join(name);
        if !from.is_file() || to.exists() {
            continue;
        }
        moved.push(*name);
        if dry_run {
            continue;
        }
        fs::create_dir_all(&target)?;
        fs::copy(&from, &to)?;
        // device.key and token.toml are secrets; carry the tight mode over
        // rather than inheriting the default umask.
        if let Ok(perms) = fs::metadata(&from).map(|m| m.permissions()) {
            let _ = fs::set_permissions(&to, perms);
        }
    }
    if moved.is_empty() {
        return Ok(());
    }
    ui::info(&format!("  migrated {} from {} (originals kept)", moved.join(", "), legacy.display()));
    Ok(())
}

fn ensure_mykey(dry_run: bool) -> Result<()> {
    let path = paths::mykey_file()?;
    let parent = path.parent().ok_or_else(|| anyhow!("bad config path: {}", path.display()))?;
    println!("  ensure {}", path.display());
    if dry_run {
        return Ok(());
    }
    fs::create_dir_all(parent)?;
    if !path.exists() {
        fs::write(&path, "")?;
    }
    Ok(())
}

// --- install / uninstall ----------------------------------------------------

fn install(prefix: &str, yes: bool, dry_run: bool) -> Result<()> {
    let prefix_dir = expand_prefix(prefix)?;
    let tools = paths::tools_dir()?;
    let release_dir = tools.join("target/release");

    ui::heading("guss tools install (from source)");
    println!("  workspace -> {}\n  prefix    -> {}", tools.display(), prefix_dir.display());
    ui::require_confirm("Build release binaries and install?", yes)?;

    proc::run_in(&tools, "cargo", &["build", "--release", "--locked"], dry_run)?;

    for (bin, cmd) in BINARIES {
        // A copy, not a symlink into target/release: a symlink dangles after
        // `cargo clean` and silently publishes whatever was last built.
        install_binary(&release_dir.join(bin), &prefix_dir.join(cmd), dry_run)?;
    }

    init(None, prefix, false, true, dry_run)
}

/// Place `src` at `dst` atomically: copy alongside, then rename over. Rename on
/// the same filesystem is atomic and works even when `dst` is the binary
/// currently executing, which matters for `setup update`.
pub(crate) fn install_binary(src: &Path, dst: &Path, dry_run: bool) -> Result<()> {
    println!("  install {} -> {}", src.display(), dst.display());
    if dry_run {
        return Ok(());
    }
    if !src.is_file() {
        return Err(anyhow!("no binary at {}", src.display()));
    }
    let parent = dst.parent().ok_or_else(|| anyhow!("bad install path: {}", dst.display()))?;
    fs::create_dir_all(parent)?;

    let staged = dst.with_extension("new");
    fs::copy(src, &staged)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&staged, dst)?;
    Ok(())
}

fn uninstall(prefix: &str, purge: bool, yes: bool, dry_run: bool) -> Result<()> {
    let prefix_dir = expand_prefix(prefix)?;
    ui::heading("guss tools uninstall");
    println!("  prefix -> {}", prefix_dir.display());
    if purge {
        println!("  purge  -> {}", config::config_dir()?.display());
    }
    ui::require_confirm(
        if purge {
            "Remove the binary, shell function, AND all config (keys, tokens)?"
        } else {
            "Remove the binary and shell function? (config is kept)"
        },
        yes,
    )?;

    for (_, cmd) in BINARIES {
        let path = prefix_dir.join(cmd);
        if path.exists() {
            println!("  remove {}", path.display());
            if !dry_run {
                fs::remove_file(&path)?;
            }
        } else {
            ui::warn(&format!("  not installed: {}", path.display()));
        }
    }

    remove_jump_function(dry_run)?;

    if purge {
        let dir = config::config_dir()?;
        if dir.is_dir() {
            println!("  remove {}", dir.display());
            if !dry_run {
                fs::remove_dir_all(&dir)?;
            }
        }
    }

    ui::good("uninstall complete");
    Ok(())
}

// --- shell function ---------------------------------------------------------

/// The rc file to edit, chosen from `$SHELL`. Previously always `~/.bashrc`, so
/// zsh users silently got no `jump`.
fn rc_file() -> Result<PathBuf> {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let name = if shell.ends_with("zsh") { ".zshrc" } else { ".bashrc" };
    Ok(paths::home()?.join(name))
}

/// The block written to the rc file, delimited so it can be removed again.
fn jump_block() -> String {
    format!("{JUMP_FN_MARKER}\n{JUMP_FN_BODY}{JUMP_FN_END}\n")
}

/// Append the `jump` shell function to the user's rc file if not already present.
fn install_jump_function(dry_run: bool) -> Result<()> {
    let rc = rc_file()?;
    let existing = fs::read_to_string(&rc).unwrap_or_default();
    if existing.contains(JUMP_FN_MARKER) {
        println!("  jump function already installed in {}", rc.display());
        return Ok(());
    }
    println!("  append jump() to {}", rc.display());
    if dry_run {
        return Ok(());
    }
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(&jump_block());
    fs::write(&rc, updated)?;
    Ok(())
}

fn remove_jump_function(dry_run: bool) -> Result<()> {
    let rc = rc_file()?;
    let existing = fs::read_to_string(&rc).unwrap_or_default();
    if !existing.contains(JUMP_FN_MARKER) {
        println!("  no jump function in {}", rc.display());
        return Ok(());
    }
    println!("  strip jump() from {}", rc.display());
    if dry_run {
        return Ok(());
    }
    fs::write(&rc, strip_jump_block(&existing))?;
    Ok(())
}

/// Remove the delimited block, leaving surrounding lines untouched. Handles the
/// pre-`JUMP_FN_END` format (marker + one line) so an older install still cleans
/// up, and is idempotent on text with no block at all.
fn strip_jump_block(text: &str) -> String {
    let mut out = String::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim() != JUMP_FN_MARKER {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Inside the block. Drop our own `jump()` line and stop at the
        // terminator; the old format had no terminator, so consuming to the end
        // of the file is correct there and preserves whatever followed.
        for inner in lines.by_ref() {
            let trimmed = inner.trim();
            if trimmed == JUMP_FN_END {
                break;
            }
            if trimmed.starts_with("jump()") {
                continue;
            }
            // Anything else means the block was hand-edited; keep it rather
            // than silently eating the user's lines.
            out.push_str(inner);
            out.push('\n');
        }
    }
    out
}

// --- misc -------------------------------------------------------------------

fn git_pull(dry_run: bool) -> Result<()> {
    let guss = paths::project_guss()?;
    for repo in REPOS {
        let dir = guss.join(repo);
        if dir.is_dir() {
            let _ = proc::run_in(&dir, "git", &["pull"], dry_run);
        } else {
            ui::warn(&format!("skip {repo}: {} not found", dir.display()));
        }
    }
    Ok(())
}

fn login_cmd(user: &str) -> Result<()> {
    ui::heading("guss login");
    println!("  logging in as {user}...");
    let token = guss_common::auth::login(user)?;
    ui::good(&format!("logged in as {} — session valid until {}", token.user, token.expires_at));
    ui::info(&format!("  allowed: {}", token.allowed.join(", ")));
    Ok(())
}

fn status_cmd() -> Result<()> {
    if !guss_common::auth::is_configured()? {
        ui::good("auth is not configured — all commands are permitted");
        ui::info(&format!(
            "  add an [auth] section to {} to turn permissions on",
            config::config_file()?.display()
        ));
        return Ok(());
    }
    match guss_common::auth::cached_token()? {
        Some(token) => {
            ui::good(&format!("logged in as {}", token.user));
            ui::info(&format!("  machine   -> {}", token.machine_id));
            ui::info(&format!("  expires   -> {}", token.expires_at));
            ui::info(&format!("  allowed   -> {}", token.allowed.join(", ")));
        }
        None => ui::warn("not logged in (or session expired) — run `guss setup login <user>`"),
    }
    Ok(())
}

fn logout_cmd() -> Result<()> {
    guss_common::auth::logout()?;
    ui::good("logged out (cached session token cleared)");
    Ok(())
}

fn print_help() {
    let text = "\
_______________________________________________________________________________
                            List of guss Admin Tools
   Run `setup init` once (or `setup install` to build from source), then call
   any task below as `guss <task> ...`.
   For things to work: put your aws credentials in ~/.aws.
_______________________________________________________________________________

guss <command>     umbrella cli — dispatches to every task below

guss net init                         set deployment mode (dev|prod|test)
guss net mode [dev|prod|test]         show or change current mode
guss net ssm <service>                open SSM session to {mode}-{service}
guss net status                       show deployment mode and network status

guss build all <dev|prod> [version]     build all apps, publish, deploy
guss build local <app> <dev|prod>       build one app, no deploy

guss deploy <cluster> <service> <dev|prod>  build + roll out one service to a cluster

guss vapi deploy <dev|prod> [server]        deploy vapi services
guss vapi doc                               regenerate the connection docs
guss vapi local start [--build] | rebuild <svc>
guss vapi tools <logs|down>                 local compose helpers

guss stack list                             registered local compose stacks
guss stack doctor [name]                    check docker/ports/env before starting
guss stack up [name] [--build]              compose up -d
guss stack rebuild [name] <svc>             rebuild + restart one service
guss stack <down|stop|start|restart|ps|logs> [name]

guss docs build [server]          regenerate vapi-docs.md
guss docs search <term>           search the docs for a route/controller

guss jump <group> [name]          cd to a project folder group (a|n|b|d|p|s|ws|v|vp)
                                 (installed as the shell function `jump`)

guss project init                          initialize git-flow branch config in this repo
guss project feature <start|finish|publish|list> <name>
guss project release <start|finish|publish> <version>
guss project hotfix <start|finish> <version>

guss setup init                   configure this machine (no cargo needed)
guss setup install                build from source and install
guss setup update [--channel]     fetch and install a newer signed release
guss setup publish <dev|prod>     build, sign, and publish a release
guss setup keygen                 generate this machine's release signing key
guss setup uninstall [--purge]    remove the binary and shell function
guss setup login <user>           log in to the central login server
guss setup status                 show the current login/session status
guss setup logout                 clear the cached session token
_______________________________________________________________________________";
    println!("{text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_expands_a_leading_tilde() {
        let home = paths::home().unwrap();
        assert_eq!(expand_prefix("~/.local/bin").unwrap(), home.join(".local/bin"));
        // No tilde: taken literally, including a bare "~" which is not a prefix.
        assert_eq!(expand_prefix("/usr/local/bin").unwrap(), PathBuf::from("/usr/local/bin"));
    }

    #[test]
    fn strip_removes_the_delimited_block_and_keeps_neighbours() {
        let rc = format!("export A=1\n{}export B=2\n", jump_block());
        assert_eq!(strip_jump_block(&rc), "export A=1\nexport B=2\n");
    }

    #[test]
    fn strip_handles_the_old_unterminated_format() {
        // Installs before JUMP_FN_END existed wrote marker + one function line.
        let rc = "export A=1\n# guss-jump\njump() { local d; d=\"$(guss jump \"$@\")\" || return; cd \"$d\"; }\nexport B=2\n";
        assert_eq!(strip_jump_block(rc), "export A=1\nexport B=2\n");
    }

    #[test]
    fn strip_is_a_no_op_without_a_block() {
        let rc = "export A=1\nexport B=2\n";
        assert_eq!(strip_jump_block(rc), rc);
    }

    #[test]
    fn install_then_strip_round_trips() {
        let rc = "export A=1\n";
        let installed = format!("{rc}{}", jump_block());
        assert!(installed.contains("jump()"));
        assert_eq!(strip_jump_block(&installed), rc);
    }

    #[test]
    fn strip_keeps_hand_edited_block_contents() {
        // If someone edited inside the markers, removing their line silently
        // would be worse than leaving it behind.
        let rc = "# guss-jump\nexport MINE=1\n# guss-jump-end\n";
        assert_eq!(strip_jump_block(rc), "export MINE=1\n");
    }
}
