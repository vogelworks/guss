//! AWS service integrations: SSM, ECR, ECS, and remote Docker operations.

pub mod ssm;
pub mod docker_remote;

use std::io::Write;
use std::process::{Command, Stdio};
use vhp_common::error::{Result, anyhow};
use vhp_common::proc;

/// Get AWS account ID (aws sts get-caller-identity --query Account --output text).
pub fn account_id() -> Result<String> {
    proc::capture("aws", &["sts", "get-caller-identity", "--query", "Account", "--output", "text"])
}

/// Get ECR registry URL: {account}.dkr.ecr.{region}.amazonaws.com
pub fn ecr_registry(account: &str, region: &str) -> String {
    format!("{}.dkr.ecr.{}.amazonaws.com", account, region)
}

/// Log in to ECR (aws ecr get-login-password | docker login --username AWS --password-stdin {registry}).
pub fn ecr_login(registry: &str, region: &str, dry_run: bool) -> Result<()> {
    println!("  $ aws ecr get-login-password --region {region} | docker login --username AWS --password-stdin {registry}");
    if dry_run {
        return Ok(());
    }

    let password = proc::capture("aws", &["ecr", "get-login-password", "--region", region])?;
    let mut child = Command::new("docker")
        .args(&["login", "--username", "AWS", "--password-stdin", registry])
        .stdin(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(password.as_bytes())?;
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("docker login failed ({})", status))
    }
}

/// Force new ECS deployment (aws ecs update-service --cluster ... --service ... --force-new-deployment).
pub fn ecs_force_deploy(cluster: &str, service: &str, region: &str, dry_run: bool) -> Result<()> {
    proc::run("aws", &[
        "ecs", "update-service",
        "--cluster", cluster,
        "--service", service,
        "--force-new-deployment",
        "--region", region,
    ], dry_run)
}
