//! Each subcommand flattens a task crate's own `Args` and dispatches to its
//! `run`, so `guss docs create <name>` runs exactly the same code as the
//! standalone `docs create <name>`. Task logic lives in the task crates;
//! this file is only wiring
//! (`guss_common::auth::enforce`) that runs before every dispatch.

use clap::{CommandFactory, Parser, Subcommand};
use guss_common::{Result, config};

#[derive(Parser)]
#[command(name = "guss", about = "guss Portal admin tools", version, propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Manage the cluster network connection (switch/bastion/status).
    Net(net::NetArgs),
    /// Build portal apps and publish into webserver.
    Build(build::BuildArgs),
    /// Build and run containers locally or deploy them (config-driven).
    Deploy(deploy::DeployArgs),
    /// Generate and search VAPI connection docs.
    Docs(docs::DocsArgs),
    /// Install the tools and bootstrap the environment.
    Setup(setup::SetupArgs),
    /// Git-flow branch controller (feature/release) for the repo in cwd.
    Project(project::ProjectArgs),
}

fn main() -> Result<()> {
    let command = Cli::parse().command;

    // If no command provided, default to interactive shell (guss)
    if let Some(cmd) = &command {
        if let Some(key) = permission_key(cmd) {
            guss_common::auth::enforce(&key)?;
        }
    }

    match command {
        Some(Command::Net(a)) => net::run(a),
        Some(Command::Build(a)) => build::run(a),
        Some(Command::Deploy(a)) => deploy::run(a),
        Some(Command::Docs(a)) => docs::run(a),
        Some(Command::Setup(a)) => setup::run(a),
        Some(Command::Project(a)) => project::run(a),
        None => guss::run(guss::GussArgs { initial_cmd: vec![] }, Cli::command()),
    }
}

/// The auth allow-list key this command needs, or `None` if it must always
/// be runnable regardless of login state (e.g. `setup login`/`status`/`logout`
/// themselves). This is the single dispatch-point gate — no per-crate checks.
///
/// Note that `enforce` fails open until `[auth]` is configured, so these keys are
/// inert on a machine with no login server — no command needs a code-level
/// exemption to stay usable.
fn permission_key(command: &Command) -> Option<String> {
    match command {
        Command::Net(_) => Some("net".to_string()),
        Command::Build(_) => Some("build".to_string()),
        Command::Deploy(_) => {
            let settings = config::Settings::load().ok()?;
            let mode = settings.get_mode().ok()?;
            Some(format!("deploy:{mode}"))
        }
        Command::Docs(_) => Some("docs".to_string()),
        Command::Setup(a) => a.permission_key(),
        Command::Project(a) => Some(a.permission_key()),
    }
}
