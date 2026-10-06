//! Docker helpers for local Docker operations (build, push, tag, prune, compose).
//! Remote Docker operations via SSM are in the aws crate.

use crate::error::{Result, anyhow};
use crate::paths;
use crate::proc;
use crate::config;
use std::path::Path;

/// Docker Hub namespace used for VHP images: `cmv343/vhp-<service>:<tag>`.
pub const REGISTRY_NS: &str = "cmv343";

/// Fully-qualified image name for a service/tag, e.g. `cmv343/vhp-webserver:dev`.
pub fn image_name(service: &str, tag: &str) -> String {
    format!("{REGISTRY_NS}/vhp-{service}:{tag}")
}

/// Build an image in `dir`, forwarding the ssh key from config (or default id_ed25519).
pub fn build(dir: &Path, image: &str, dry_run: bool) -> Result<()> {
    build_with_context(dir, image, ".", dry_run)
}

/// Build an image in `dir` with a custom build context (e.g., `../` for parent directory).
pub fn build_with_context(dir: &Path, image: &str, context: &str, dry_run: bool) -> Result<()> {
    build_with_context_and_args(dir, image, context, &[], dry_run, false)
}

/// Build an image with custom build context and build arguments.
/// `args` should be key=value pairs like ["CONFIG_PATH=../config.json"]
pub fn build_with_context_and_args(
    dir: &Path,
    image: &str,
    context: &str,
    args: &[&str],
    dry_run: bool,
    no_cache: bool,
) -> Result<()> {
    build_with_dockerfile_and_args(dir, image, context, "Dockerfile", args, dry_run, no_cache)
}

pub fn build_with_dockerfile_and_args(
    dir: &Path,
    image: &str,
    context: &str,
    dockerfile: &str,
    args: &[&str],
    dry_run: bool,
    no_cache: bool,
) -> Result<()> {
    let settings = config::Settings::load()?;
    let ssh_key_name = settings.get_ssh_key();
    let ssh_key = paths::home()?.join(".ssh").join(&ssh_key_name);
    let ssh_arg = format!("default={}", ssh_key.display());

    let mut cmd = vec!["build"];
    if no_cache {
        cmd.push("--no-cache");
    }
    cmd.extend(&["--ssh", &ssh_arg, "-f", dockerfile]);
    for arg in args {
        cmd.push("--build-arg");
        cmd.push(arg);
    }
    cmd.extend(&["-t", image, context]);

    proc::run_in(dir, "docker", &cmd, dry_run)
}

/// `docker tag <src> <dst>`.
pub fn tag(src: &str, dst: &str, dry_run: bool) -> Result<()> {
    proc::run("docker", &["tag", src, dst], dry_run)
}

/// `docker push <image>`.
pub fn push(image: &str, dry_run: bool) -> Result<()> {
    proc::run("docker", &["push", image], dry_run)
}

/// `docker system prune -af` (local).
pub fn prune(dry_run: bool) -> Result<()> {
    proc::run("docker", &["system", "prune", "-af"], dry_run)
}

/// `docker compose -f <file> -p <project> up -d` (local).
pub fn compose_up(compose: &Path, project: &str, dry_run: bool) -> Result<()> {
    let root = paths::project_vhp()?;
    proc::run_in(
        &root,
        "sudo",
        &[
            "docker",
            "compose",
            "-f",
            compose.to_str().ok_or_else(|| anyhow!("invalid compose path"))?,
            "-p",
            project,
            "up",
            "-d",
        ],
        dry_run,
    )
}

/// `docker compose -f <file> -p <project> down` (local).
pub fn compose_down(compose: &Path, project: &str, dry_run: bool) -> Result<()> {
    let root = paths::project_vhp()?;
    proc::run_in(
        &root,
        "sudo",
        &[
            "docker",
            "compose",
            "-f",
            compose.to_str().ok_or_else(|| anyhow!("invalid compose path"))?,
            "-p",
            project,
            "down",
        ],
        dry_run,
    )
}

/// `docker compose -f <file> -p <project> stop` (local).
pub fn compose_stop(compose: &Path, project: &str, dry_run: bool) -> Result<()> {
    let root = paths::project_vhp()?;
    proc::run_in(
        &root,
        "sudo",
        &[
            "docker",
            "compose",
            "-f",
            compose.to_str().ok_or_else(|| anyhow!("invalid compose path"))?,
            "-p",
            project,
            "stop",
        ],
        dry_run,
    )
}

/// `docker compose -f <file> -p <project> start` (local).
pub fn compose_start(compose: &Path, project: &str, dry_run: bool) -> Result<()> {
    let root = paths::project_vhp()?;
    proc::run_in(
        &root,
        "sudo",
        &[
            "docker",
            "compose",
            "-f",
            compose.to_str().ok_or_else(|| anyhow!("invalid compose path"))?,
            "-p",
            project,
            "start",
        ],
        dry_run,
    )
}

/// `docker compose -f <file> -p <project> ps` (local).
pub fn compose_ps(compose: &Path, project: &str, dry_run: bool) -> Result<()> {
    let root = paths::project_vhp()?;
    proc::run_in(
        &root,
        "sudo",
        &[
            "docker",
            "compose",
            "-f",
            compose.to_str().ok_or_else(|| anyhow!("invalid compose path"))?,
            "-p",
            project,
            "ps",
        ],
        dry_run,
    )
}

/// `docker compose -f <file> -p <project> logs` with optional tail (local).
pub fn compose_logs(compose: &Path, project: &str, tail: Option<usize>, dry_run: bool) -> Result<()> {
    let root = paths::project_vhp()?;
    let compose_str = compose.to_str().ok_or_else(|| anyhow!("invalid compose path"))?;
    let tail_str = tail.map(|n| n.to_string());
    let mut cmd = vec!["docker", "compose", "-f", compose_str, "-p", project, "logs"];
    if let Some(n) = &tail_str {
        cmd.push("--tail");
        cmd.push(n);
    }
    cmd.push("-f");
    proc::run_in(&root, "sudo", &cmd, dry_run)
}

