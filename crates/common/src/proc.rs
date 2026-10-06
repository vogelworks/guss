//! Subprocess helpers, generalized from `project`'s `git.rs`/`gh.rs`.
//!
//! All task crates shell out to `git`, `docker`, `ssh`, `npm`, etc. These helpers
//! give one consistent behavior: echo the command, run it, and turn a non-zero
//! exit into an `Err`. `dry_run` variants print the command instead of running it.

use crate::error::{Result, anyhow};
use std::path::Path;
use std::process::Command;

/// Format a command for display, e.g. `git commit -m ...`.
fn render(program: &str, args: &[&str]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{} {}", program, args.join(" "))
    }
}

/// Run a command, streaming its stdout/stderr to the terminal. Errors on non-zero
/// exit. When `dry_run` is true, only prints the command and returns `Ok`.
pub fn run(program: &str, args: &[&str], dry_run: bool) -> Result<()> {
    println!("  $ {}", render(program, args));
    if dry_run {
        return Ok(());
    }
    let status = Command::new(program).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("`{}` failed ({})", render(program, args), status))
    }
}

/// Like [`run`], but sets `envs` on the child process.
///
/// Values are masked in the echoed line (`KEY=***`) — these carry shared secrets
/// and must not land in scrollback or CI logs. Note that `sudo` scrubs the
/// environment by default, so a `sudo` invocation needs a matching
/// `--preserve-env=<names>` for these to reach the real program.
pub fn run_env(program: &str, args: &[&str], envs: &[(String, String)], dry_run: bool) -> Result<()> {
    let masked = envs
        .iter()
        .map(|(k, _)| format!("{k}=***"))
        .collect::<Vec<_>>()
        .join(" ");
    let rendered = render(program, args);
    if masked.is_empty() {
        println!("  $ {rendered}");
    } else {
        println!("  $ {masked} {rendered}");
    }
    if dry_run {
        return Ok(());
    }
    let status = Command::new(program)
        .args(args)
        .envs(envs.iter().map(|(k, v)| (k, v)))
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("`{rendered}` failed ({status})"))
    }
}

/// Like [`run`], but executes in `dir`.
pub fn run_in(dir: &Path, program: &str, args: &[&str], dry_run: bool) -> Result<()> {
    println!("  $ ({}) {}", dir.display(), render(program, args));
    if dry_run {
        return Ok(());
    }
    let status = Command::new(program).args(args).current_dir(dir).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("`{}` failed ({})", render(program, args), status))
    }
}

/// Run a command and capture trimmed stdout. Errors on non-zero exit.
/// Always executes (never skipped) — reads are safe in dry-run mode.
pub fn capture(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(anyhow!("`{}` failed: {}", render(program, args), stderr))
    }
}

/// Like [`capture`], but executes in `dir`.
pub fn capture_in(dir: &Path, program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).current_dir(dir).output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(anyhow!("`{}` failed: {}", render(program, args), stderr))
    }
}

/// Capture stdout, returning an empty string on any failure. Mirrors
/// `gust`'s `run_optional` — useful for `git branch --list` style probes.
pub fn capture_optional(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Return true if a program exists and the given probe args exit successfully
/// (e.g. `gh auth status`, `docker version`).
pub fn succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
