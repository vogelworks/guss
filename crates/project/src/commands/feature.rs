use crate::config::Config;
use crate::gh;
use crate::git;
use crate::hooks;
use colored::Colorize;

use guss_common::{Result, error::anyhow};

/// guss project feature start <name>
/// Creates a new feature branch off develop.
pub fn start(config: &Config, name: &str) -> Result<()> {
    let branch = config.feature_branch(name);

    if git::branch_exists(&branch) {
        return Err(anyhow!(format!("feature branch '{}' already exists.", branch)));
    }

    if !git::is_clean() {
        return Err(anyhow!("working tree is not clean. Commit or stash your changes first."));
    }

    git::run(&["checkout", &config.branch.develop])?;
    git::run(&["checkout", "-b", &branch])?;

    println!("{} {}", "Created and switched to feature branch:".green(), branch.cyan());
    println!("Now make your changes, then run:");
    println!("  guss project feature finish {}", name);

    hooks::run(config, "feature.start", &branch)?;
    Ok(())
}

/// guss project feature finish <name>
/// Merges the feature branch back into develop and deletes it.
pub fn finish(config: &Config, name: &str) -> Result<()> {
    let branch = config.feature_branch(name);

    if !git::branch_exists(&branch) {
        return Err(anyhow!(format!("feature branch '{}' does not exist.", branch)));
    }

    if !git::is_clean() {
        return Err(anyhow!("working tree is not clean. Commit or stash your changes first."));
    }

    git::run(&["checkout", &config.branch.develop])?;
    // --no-ff preserves the branch topology in history
    git::run(&["merge", "--no-ff", &branch, "-m", &format!("Merge branch '{}'", branch)])?;
    git::run(&["branch", "-d", &branch])?;

    println!(
        "{} {} {} {}",
        "Merged".green(),
        branch.cyan(),
        "into".green(),
        config.branch.develop.cyan()
    );

    hooks::run(config, "feature.finish", &branch)?;
    Ok(())
}

/// guss project feature publish <name>
/// Pushes the feature branch to remote and optionally opens a PR into develop.
pub fn publish(config: &Config, name: &str) -> Result<()> {
    let branch = config.feature_branch(name);

    if !git::branch_exists(&branch) {
        return Err(anyhow!(format!("feature branch '{}' does not exist.", branch)));
    }

    git::run(&["push", "origin", &branch])?;
    println!("{} {}", "Published feature branch:".green(), branch.cyan());

    if config.pr.enabled {
        if !gh::is_available() {
            println!("{}", "warning: gh CLI not found or not authenticated — skipping PR creation.".yellow());
        } else {
            let url = gh::create_pr(&branch, &config.branch.develop)?;
            println!("{} {}", "Pull request opened:".green(), url.cyan());
        }
    }

    hooks::run(config, "feature.publish", &branch)?;
    Ok(())
}

/// guss project feature list
/// Lists all local feature branches.
pub fn list(config: &Config) {
    let pattern = format!("{}*", config.prefix.feature);
    let out = git::run_optional(&["branch", "--list", &pattern]);

    if out.is_empty() {
        println!("No feature branches found.");
    } else {
        println!("{}", "Feature branches:".yellow());
        for line in out.lines() {
            println!("  {}", line.trim().cyan());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{DIR_LOCK, OrigDir, TestRepo};

    #[test]
    fn start_creates_branch() {
        let _lock = DIR_LOCK.lock().unwrap();
        let repo = TestRepo::new("feat_start");
        let _orig = OrigDir::save_and_switch(&repo.dir);

        let config = Config::load().unwrap();
        start(&config, "login").unwrap();

        assert!(git::branch_exists("feature/login"));
        assert_eq!(git::current_branch().unwrap(), "feature/login");
    }

    #[test]
    fn start_fails_if_branch_already_exists() {
        let _lock = DIR_LOCK.lock().unwrap();
        let repo = TestRepo::new("feat_start_dup");
        let _orig = OrigDir::save_and_switch(&repo.dir);

        let config = Config::load().unwrap();
        start(&config, "dupe").unwrap();
        // return to develop so the working tree is clean for the second call
        repo.git(&["checkout", "develop"]);

        let err = start(&config, "dupe").unwrap_err().to_string();
        assert!(err.contains("already exists"), "got: {}", err);
    }

    #[test]
    fn finish_merges_into_develop_and_deletes_branch() {
        let _lock = DIR_LOCK.lock().unwrap();
        let repo = TestRepo::new("feat_finish");
        let _orig = OrigDir::save_and_switch(&repo.dir);

        let config = Config::load().unwrap();
        start(&config, "widget").unwrap();
        repo.empty_commit("Add widget");

        finish(&config, "widget").unwrap();

        assert!(!git::branch_exists("feature/widget"));
        assert_eq!(git::current_branch().unwrap(), "develop");
    }

    #[test]
    fn finish_runs_configured_hook() {
        let _lock = DIR_LOCK.lock().unwrap();
        let repo = TestRepo::new("feat_finish_hook");
        let _orig = OrigDir::save_and_switch(&repo.dir);

        let mut config = Config::load().unwrap();
        let marker = repo.dir.join("hook-ran");
        config.hooks.push(crate::config::HookConfig {
            event: "feature.finish".to_string(),
            branch: Some("feature/*".to_string()),
            run: format!("touch {}", marker.display()),
        });

        start(&config, "gadget").unwrap();
        repo.empty_commit("Add gadget");
        finish(&config, "gadget").unwrap();

        assert!(marker.exists(), "expected hook to create marker file");
    }
}
