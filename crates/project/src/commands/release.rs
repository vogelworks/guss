use crate::config::Config;
use crate::gh;
use crate::git;
use crate::hooks;
use colored::Colorize;

use guss_common::{Result, error::anyhow};

/// guss project release start <version>
/// Creates a release branch off develop.
pub fn start(config: &Config, version: &str) -> Result<()> {
    let branch = config.release_branch(version);

    if git::branch_exists(&branch) {
        return Err(anyhow!(format!("release branch '{}' already exists.", branch)));
    }

    if !git::is_clean() {
        return Err(anyhow!("working tree is not clean. Commit or stash your changes first."));
    }

    git::run(&["checkout", &config.branch.develop])?;
    git::run(&["checkout", "-b", &branch])?;

    println!("{} {}", "Created and switched to release branch:".green(), branch.cyan());
    println!("Bump version numbers, update changelogs, then run:");
    println!("  guss project release finish {}", version);

    hooks::run(config, "release.start", &branch)?;
    Ok(())
}

/// guss project release finish <version>
/// Merges release into main and develop, tags main, deletes branch.
pub fn finish(config: &Config, version: &str) -> Result<()> {
    let branch = config.release_branch(version);
    let tag = config.release_tag(version);

    if !git::branch_exists(&branch) {
        return Err(anyhow!(format!("release branch '{}' does not exist.", branch)));
    }

    if !git::is_clean() {
        return Err(anyhow!("working tree is not clean. Commit or stash your changes first."));
    }

    // Merge into main
    git::run(&["checkout", &config.branch.main])?;
    git::run(&["merge", "--no-ff", &branch, "-m", &format!("Merge branch '{}'", branch)])?;

    // Tag the release on main
    git::run(&["tag", "-a", &tag, "-m", &format!("Release {}", version)])?;

    // Merge into develop
    git::run(&["checkout", &config.branch.develop])?;
    git::run(&["merge", "--no-ff", &branch, "-m", &format!("Merge branch '{}'", branch)])?;

    // Delete the release branch
    git::run(&["branch", "-d", &branch])?;

    println!(
        "{} {} {} {} {} {}",
        "Merged".green(),
        branch.cyan(),
        "into".green(),
        config.branch.main.cyan(),
        "and".green(),
        config.branch.develop.cyan()
    );
    println!("{} {}", "Tagged release:".green(), tag.cyan());

    hooks::run(config, "release.finish", &branch)?;
    Ok(())
}

/// guss project release publish <version>
/// Pushes the release branch to remote and optionally opens a PR into main.
pub fn publish(config: &Config, version: &str) -> Result<()> {
    let branch = config.release_branch(version);

    if !git::branch_exists(&branch) {
        return Err(anyhow!(format!("release branch '{}' does not exist.", branch)));
    }

    git::run(&["push", "origin", &branch])?;
    println!("{} {}", "Published release branch:".green(), branch.cyan());

    if config.pr.enabled {
        if !gh::is_available() {
            println!("{}", "warning: gh CLI not found or not authenticated — skipping PR creation.".yellow());
        } else {
            let url = gh::create_pr(&branch, &config.branch.main)?;
            println!("{} {}", "Pull request opened:".green(), url.cyan());
        }
    }

    hooks::run(config, "release.publish", &branch)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{DIR_LOCK, OrigDir, TestRepo};

    #[test]
    fn start_creates_branch() {
        let _lock = DIR_LOCK.lock().unwrap();
        let repo = TestRepo::new("rel_start");
        let _orig = OrigDir::save_and_switch(&repo.dir);

        let config = Config::load().unwrap();
        start(&config, "1.0.0").unwrap();

        assert!(git::branch_exists("release/1.0.0"));
        assert_eq!(git::current_branch().unwrap(), "release/1.0.0");
    }

    #[test]
    fn start_fails_if_branch_already_exists() {
        let _lock = DIR_LOCK.lock().unwrap();
        let repo = TestRepo::new("rel_start_dup");
        let _orig = OrigDir::save_and_switch(&repo.dir);

        let config = Config::load().unwrap();
        start(&config, "2.0.0").unwrap();
        repo.git(&["checkout", "develop"]);

        let err = start(&config, "2.0.0").unwrap_err().to_string();
        assert!(err.contains("already exists"), "got: {}", err);
    }

    #[test]
    fn finish_merges_into_main_tags_and_deletes_branch() {
        let _lock = DIR_LOCK.lock().unwrap();
        let repo = TestRepo::new("rel_finish");
        let _orig = OrigDir::save_and_switch(&repo.dir);

        let config = Config::load().unwrap();
        start(&config, "1.2.0").unwrap();
        repo.empty_commit("Bump version");

        finish(&config, "1.2.0").unwrap();

        assert!(!git::branch_exists("release/1.2.0"));
        assert_eq!(git::current_branch().unwrap(), "develop");

        let tags = git::run_optional(&["tag", "--list", "1.2.0"]);
        assert_eq!(tags.trim(), "1.2.0", "expected tag 1.2.0 to exist");
    }
}
