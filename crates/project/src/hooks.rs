//! Runs configured post-action hooks (`[[hooks]]` in `.gitflow`).
//!
//! Hooks only fire through `guss project` commands — there's no real git hook
//! installed, so a bare `git push`/`git pull` in the repo bypasses them.

use colored::Colorize;
use guss_common::{Result, error::anyhow};

use crate::config::Config;

/// Run every hook matching `event` and `branch`, in config order.
/// Stops and returns an error on the first hook that exits non-zero.
pub fn run(config: &Config, event: &str, branch: &str) -> Result<()> {
    for hook in &config.hooks {
        if hook.event != event {
            continue;
        }
        if !glob_match(hook.branch.as_deref().unwrap_or("*"), branch) {
            continue;
        }

        println!("{} {} {}", "hook:".yellow(), event.cyan(), hook.run);
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(&hook.run)
            .status()
            .map_err(|e| anyhow!("failed to run hook `{}`: {}", hook.run, e))?;
        if !status.success() {
            return Err(anyhow!("hook `{}` for event `{}` exited with {}", hook.run, event, status));
        }
    }
    Ok(())
}

/// Minimal glob match: `*` as a wildcard for any number of characters,
/// no other special characters. `pattern == "*"` always matches.
fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }

    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !rest.starts_with(part) {
                return false;
            }
            rest = &rest[part.len()..];
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(idx) => rest = &rest[idx + part.len()..],
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_star_matches_anything() {
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "release/1.0.0"));
    }

    #[test]
    fn glob_exact_match() {
        assert!(glob_match("develop", "develop"));
        assert!(!glob_match("develop", "development"));
    }

    #[test]
    fn glob_prefix_wildcard() {
        assert!(glob_match("release/*", "release/1.0.0"));
        assert!(!glob_match("release/*", "feature/1.0.0"));
    }

    #[test]
    fn glob_suffix_and_infix_wildcard() {
        assert!(glob_match("*-hotfix", "urgent-hotfix"));
        assert!(glob_match("release/*-rc", "release/2.0.0-rc"));
        assert!(!glob_match("release/*-rc", "release/2.0.0"));
    }

    fn cfg_with_hook(event: &str, branch: Option<&str>, run: &str) -> Config {
        let mut c = Config::default();
        c.hooks.push(crate::config::HookConfig {
            event: event.to_string(),
            branch: branch.map(str::to_string),
            run: run.to_string(),
        });
        c
    }

    #[test]
    fn runs_matching_hook() {
        let dir = std::env::temp_dir().join(format!(
            "gust_hook_test_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        let marker = dir.join("ran");
        std::fs::create_dir_all(&dir).unwrap();

        let config = cfg_with_hook("feature.publish", Some("feature/*"), &format!("touch {}", marker.display()));

        run(&config, "feature.publish", "feature/login").unwrap();
        assert!(marker.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_non_matching_event() {
        let config = cfg_with_hook("release.publish", None, "exit 1");
        // Would fail (exit 1) if it ran; since the event doesn't match, it's skipped.
        run(&config, "feature.publish", "feature/login").unwrap();
    }

    #[test]
    fn skips_non_matching_branch() {
        let config = cfg_with_hook("feature.publish", Some("release/*"), "exit 1");
        run(&config, "feature.publish", "feature/login").unwrap();
    }

    #[test]
    fn propagates_hook_failure() {
        let config = cfg_with_hook("feature.publish", None, "exit 3");
        let err = run(&config, "feature.publish", "feature/login").unwrap_err();
        assert!(err.to_string().contains("feature.publish"), "got: {}", err);
    }
}
