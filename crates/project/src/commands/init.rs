use crate::config::Config;
use crate::git;
use colored::Colorize;

use vhp_common::{Result, error::anyhow};

pub fn run() -> Result<()> {
    if !git::is_repo() {
        return Err(anyhow!("not a git repository. Run `git init` first."));
    }

    // If .gitflow already exists, warn and exit
    if std::path::Path::new(".gitflow").exists() {
        return Err(anyhow!("git-flow is already initialized in this repository."));
    }

    let config = Config::default();

    // Ensure the main branch exists
    let current = git::current_branch()?;
    if current != config.branch.main && !git::branch_exists(&config.branch.main) {
        println!("{}", "Creating main branch...".yellow());
        git::run(&["checkout", "-b", &config.branch.main])?;
    } else if current != config.branch.main {
        git::run(&["checkout", &config.branch.main])?;
    }

    // Ensure the develop branch exists
    if !git::branch_exists(&config.branch.develop) {
        println!("{}", "Creating develop branch...".yellow());
        git::run(&["checkout", "-b", &config.branch.develop])?;
    } else {
        git::run(&["checkout", &config.branch.develop])?;
    }

    config.save()?;

    println!("{}", "Initialized git-flow repository.".green());
    println!("  Main branch:    {}", config.branch.main.cyan());
    println!("  Develop branch: {}", config.branch.develop.cyan());
    println!("  Feature prefix: {}", config.prefix.feature.cyan());
    println!("  Release prefix: {}", config.prefix.release.cyan());
    println!("  Hotfix prefix:  {}", config.prefix.hotfix.cyan());

    Ok(())
}
