//! Remote Docker operations via AWS Systems Manager on net cluster instances.

use vhp_common::error::Result;
use vhp_common::config::Mode;
use vhp_common::ui;
use crate::ssm;

/// Pull an image on a remote host via SSM: run `sudo docker pull <image>` on `{mode}-{service}`.
pub fn pull(mode: Mode, service: &str, image: &str, yes: bool, dry_run: bool) -> Result<()> {
    let cmd = format!("sudo docker pull {image}");
    if mode == Mode::Prod {
        ui::require_confirm(&format!("{} on {}-{} ({})?", cmd, mode, service, "prod"), yes)?;
    }
    if dry_run {
        println!("  $ {cmd} (on {mode}-{service})");
        Ok(())
    } else {
        ssm::run_command(mode, service, &cmd)?;
        Ok(())
    }
}

/// Bring a service up remotely via SSM: run `sudo docker-compose ... up -d <service>`.
pub fn compose_up(
    mode: Mode,
    service: &str,
    cluster: &str,
    compose_service: &str,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    let project = format!("{cluster}-cluster");
    let compose = format!("/etc/docker/compose/{cluster}-cluster/docker-compose.yml");
    let cmd = format!(
        "sudo docker-compose -p {project} -f {compose} up -d {compose_service}"
    );
    if mode == Mode::Prod {
        ui::require_confirm(&format!("{} on {}-{} ({})?", cmd, mode, service, "prod"), yes)?;
    }
    if dry_run {
        println!("  $ {cmd} (on {mode}-{service})");
        Ok(())
    } else {
        ssm::run_command(mode, service, &cmd)?;
        Ok(())
    }
}

/// Prune Docker on a remote host via SSM: run `sudo docker system prune -af` on `{mode}-{service}`.
pub fn prune(mode: Mode, service: &str, yes: bool, dry_run: bool) -> Result<()> {
    let cmd = "sudo docker system prune -af";
    if mode == Mode::Prod {
        ui::require_confirm(&format!("{} on {}-{} ({})?", cmd, mode, service, "prod"), yes)?;
    }
    if dry_run {
        println!("  $ {cmd} (on {mode}-{service})");
        Ok(())
    } else {
        ssm::run_command(mode, service, cmd)?;
        Ok(())
    }
}
