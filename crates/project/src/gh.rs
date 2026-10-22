//! GitHub CLI (`gh`) helpers for the `project` task, built on `guss_common::proc`.

use guss_common::Result;
use guss_common::proc;

/// Return true if the `gh` CLI is installed and authenticated.
pub fn is_available() -> bool {
    proc::succeeds("gh", &["auth", "status"])
}

/// Open a pull request via the `gh` CLI.
/// Uses --fill to populate title/body from commit messages.
pub fn create_pr(head: &str, base: &str) -> Result<String> {
    println!("  gh pr create --head {head} --base {base} --fill");
    proc::capture("gh", &["pr", "create", "--head", head, "--base", base, "--fill"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_pr_returns_err_when_gh_unavailable() {
        if is_available() {
            println!("skipped: gh is available in this environment");
            return;
        }
        let result = create_pr("feature/test-branch", "develop");
        assert!(result.is_err(), "expected Err when gh is not available");
    }
}
