//! Git helpers shared by `project` (git-flow) and the VHP tasks (submodule
//! branch switching in `net`/`build`/`deploy`). Originally `gust`'s standalone
//! `git.rs`, made directory-aware and dry-run-aware.

use crate::error::{Result, anyhow};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// Run a git command in `dir`, echoing it, and return trimmed stdout.
/// Errors on non-zero exit. When `dry_run`, prints and returns an empty string.
pub fn run(dir: &Path, args: &[&str], dry_run: bool) -> Result<String> {
    println!("  git {}", args.join(" "));
    if dry_run {
        return Ok(String::new());
    }
    let output = Command::new("git").args(args).current_dir(dir).output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(anyhow!("git {}: {}", args.join(" "), stderr))
    }
}

/// Run a git command in `dir` and return stdout, empty on any failure.
/// Useful for `git branch --list` style probes where "no match" is valid.
pub fn run_optional(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Name of the current branch in `dir`.
pub fn current_branch(dir: &Path) -> Result<String> {
    // Always executes: a read that is safe even in dry-run mode.
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(anyhow!(
            "not a git repository: {}",
            dir.display()
        ))
    }
}

/// True if the given branch exists locally in `dir`.
pub fn branch_exists(dir: &Path, name: &str) -> bool {
    !run_optional(dir, &["branch", "--list", name]).is_empty()
}

/// True if the given remote branch exists in `dir`.
pub fn remote_branch_exists(dir: &Path, remote: &str, name: &str) -> bool {
    let full = format!("{remote}/{name}");
    !run_optional(dir, &["branch", "-r", "--list", &full]).is_empty()
}

/// True if the working tree in `dir` is clean.
pub fn is_clean(dir: &Path) -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .map(|o| o.stdout.is_empty())
        .unwrap_or(false)
}

/// True if `dir` is a git repository root (has .git directory or file for submodules).
pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Require the git repo at `dir` to be on the branch `mode` demands: dev/prod
/// need an exact match; test allows any branch except "prod" (test mode never
/// forces a switch away from whatever non-prod branch is checked out).
/// No-op if `dir` isn't a repo.
pub fn require_mode_branch(dir: &Path, mode: crate::config::Mode) -> Result<()> {
    if !is_repo(dir) {
        return Ok(());
    }
    let branch = current_branch(dir)?;
    if mode == crate::config::Mode::Test {
        if branch == "prod" {
            return Err(anyhow!(
                "{} is on '{branch}' (production branch). In test mode, use a different branch.",
                dir.display()
            ));
        }
        return Ok(());
    }
    let want = mode.to_string();
    if branch != want {
        return Err(anyhow!(
            "{} is on '{branch}', need '{want}'. Run `git checkout {want}` first.",
            dir.display()
        ));
    }
    Ok(())
}

/// Install an executable git hook (e.g. `pre-commit`) in `dir`'s `.git/hooks`.
/// Used by `project` to wire up repo hooks.
pub fn install_hook(dir: &Path, name: &str, body: &str) -> Result<()> {
    let hooks = run(dir, &["rev-parse", "--git-path", "hooks"], false)?;
    let hooks_dir = dir.join(hooks);
    fs::create_dir_all(&hooks_dir)?;
    let hook = hooks_dir.join(name);
    fs::write(&hook, body)?;
    let mut perms = fs::metadata(&hook)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&hook, perms)?;
    Ok(())
}
