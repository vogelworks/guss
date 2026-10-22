//! Thin cwd-based git helpers for the `project` task, delegating to
//! `guss_common::git`. `guss project` always operates on the current
//! repository, so these wrap the directory-aware shared helpers with `.`
//! and no dry-run.

use guss_common::Result;
use std::path::Path;

fn here() -> &'static Path {
    Path::new(".")
}

/// Run a git command and return trimmed stdout. Prints the command first.
pub fn run(args: &[&str]) -> Result<String> {
    guss_common::git::run(here(), args, false)
}

/// Run a git command and return stdout, even on non-zero exit.
pub fn run_optional(args: &[&str]) -> String {
    guss_common::git::run_optional(here(), args)
}

/// Return the name of the current branch.
pub fn current_branch() -> Result<String> {
    guss_common::git::current_branch(here())
}

/// Return true if the given branch exists locally.
pub fn branch_exists(name: &str) -> bool {
    guss_common::git::branch_exists(here(), name)
}

/// Return true if the working tree is clean.
pub fn is_clean() -> bool {
    guss_common::git::is_clean(here())
}

/// Return true if we are inside a git repository.
pub fn is_repo() -> bool {
    guss_common::git::is_repo(here())
}
