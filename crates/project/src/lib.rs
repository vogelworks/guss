//! `project` task: git-flow branch controller (feature/release), wired
//! in as `vhp project ...`. Operates on the repo in the current directory —
//! unrelated to which VHP repo you're standing in, unlike the other tasks.

mod commands;
mod config;
mod gh;
mod git;
mod hooks;
#[cfg(test)]
mod test_helpers;

use clap::{Args, Subcommand};
use config::Config;

use vhp_common::Result;

#[derive(Args)]
pub struct ProjectArgs {
    #[command(subcommand)]
    command: Command,
}

impl ProjectArgs {
    /// Permission key for the auth allow-list. Release touches shared
    /// branches/tags, so it's qualified separately from everything else in
    /// this crate.
    pub fn permission_key(&self) -> String {
        match &self.command {
            Command::Release { .. } => "project:release".to_string(),
            _ => "project".to_string(),
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Initialize git-flow branch config in this repository
    Init,
    /// Manage feature branches
    Feature {
        #[command(subcommand)]
        subcommand: FeatureCmd,
    },
    /// Manage release branches
    Release {
        #[command(subcommand)]
        subcommand: ReleaseCmd,
    },
}

#[derive(Subcommand)]
enum FeatureCmd {
    /// Start a new feature branch
    Start { name: String },
    /// Merge feature back into develop
    Finish { name: String },
    /// Push feature branch to remote
    Publish { name: String },
    /// List all feature branches
    List,
}

#[derive(Subcommand)]
enum ReleaseCmd {
    /// Start a release branch
    Start { version: String },
    /// Merge release into main and develop
    Finish { version: String },
    /// Push release branch to remote
    Publish { version: String },
}

pub fn run(args: ProjectArgs) -> Result<()> {
    match args.command {
        Command::Init => {
            commands::init::run()?;
        }

        Command::Feature { subcommand } => {
            let config = Config::load()?;
            match subcommand {
                FeatureCmd::Start { name } => commands::feature::start(&config, &name)?,
                FeatureCmd::Finish { name } => commands::feature::finish(&config, &name)?,
                FeatureCmd::Publish { name } => commands::feature::publish(&config, &name)?,
                FeatureCmd::List => commands::feature::list(&config),
            }
        }

        Command::Release { subcommand } => {
            let config = Config::load()?;
            match subcommand {
                ReleaseCmd::Start { version } => commands::release::start(&config, &version)?,
                ReleaseCmd::Finish { version } => commands::release::finish(&config, &version)?,
                ReleaseCmd::Publish { version } => commands::release::publish(&config, &version)?,
            }
        }
    }

    Ok(())
}
