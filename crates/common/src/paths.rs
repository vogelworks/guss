//! Canonical filesystem paths for the guss layout.
//!
//! Hard-coded `/home/$USER/project-guss/...` and `~/project-vapi/...` strings were
//! scattered across every shell script; they live here now. The monorepo root
//! itself is resolved by [`project_guss`] rather than hard-coded, so a checkout
//! that is not at `~/project-guss` still works.
//!
//! Two distinct kinds of path live here, and the difference matters:
//!
//! * **repo paths** ([`tools_dir`], [`network_dir`], [`applications_dir`], …) hang
//!   off the checkout, wherever that is;
//! * **config paths** ([`mykey_file`], [`token_file`], [`stacks_file`], …) hang off
//!   `~/.config/guss` and are deliberately *outside* any checkout, so they survive
//!   `git clean`, are shared between checkouts, and do not depend on where the
//!   code happens to live.

use crate::config;
use crate::error::{Result, anyhow};
use std::path::{Path, PathBuf};

/// Env var that pins the monorepo root, bypassing the probe in [`project_guss`].
pub const ROOT_ENV: &str = "guss_ROOT";

/// Checkout locations tried, in order, when `guss_ROOT` is unset.
const ROOT_CANDIDATES: &[&str] = &["project-guss", "guss/project-guss"];

/// The marker that identifies a real checkout rather than a leftover directory
/// (`~/project-guss` can exist holding only `-config/`).
const ROOT_MARKER: &str = "tools/Cargo.toml";

/// The user's home directory (`$HOME`).
pub fn home() -> Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| anyhow!("HOME is not set"))
}

/// The monorepo root, in precedence order:
///
/// 1. `$guss_ROOT` — the one-off escape hatch;
/// 2. `root` in `~/.config/guss/config.toml` — the normal mechanism, written by
///    `setup init`, which is what lets each teammate keep their checkout wherever
///    they like;
/// 3. the [`ROOT_CANDIDATES`] probe — last resort, so a machine with no config
///    still works. The [`ROOT_MARKER`] check means a stale `~/project-guss` holding
///    only `-config/` does not shadow a real `~/guss/project-guss`.
pub fn project_guss() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os(ROOT_ENV) {
        let root = PathBuf::from(root);
        if root.as_os_str().is_empty() {
            return Err(anyhow!("{ROOT_ENV} is set but empty"));
        }
        return Ok(root);
    }
    if let Some(root) = config::Settings::load()?.root {
        return Ok(root);
    }
    let home = home()?;
    resolve_root(&home).ok_or_else(|| {
        let tried = ROOT_CANDIDATES
            .iter()
            .map(|c| home.join(c).display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow!(
            "no project-guss checkout found (tried {tried}); run `guss setup init`, \
             or set {ROOT_ENV} to its path"
        )
    })
}

/// First candidate under `home` that contains [`ROOT_MARKER`]. Split out so the
/// probe order is testable without touching `$HOME`.
fn resolve_root(home: &Path) -> Option<PathBuf> {
    ROOT_CANDIDATES.iter().map(|c| home.join(c)).find(|dir| dir.join(ROOT_MARKER).exists())
}

/// The backend API services tree scanned by the docs builder.
///
/// `vapi_root` from config if set, else `~/project-vapi`. The config hook exists
/// because the services now live in the `servers` submodule of the checkout on at
/// least some machines, where the old default points at nothing.
pub fn project_vapi() -> Result<PathBuf> {
    if let Some(root) = config::Settings::load()?.vapi_root {
        return Ok(root);
    }
    Ok(home()?.join("project-vapi"))
}

/// `~/project-guss/tools` — the Rust workspace root (where `setup` runs cargo).
pub fn tools_dir() -> Result<PathBuf> {
    Ok(project_guss()?.join("tools"))
}

/// `~/project-guss/network` (the `vn-network-services` submodule).
pub fn network_dir() -> Result<PathBuf> {
    Ok(project_guss()?.join("network"))
}

/// `~/project-guss/applications` (the `project-portal` submodule).
pub fn applications_dir() -> Result<PathBuf> {
    Ok(project_guss()?.join("applications"))
}

/// `~/project-guss/webserver` (the `vs-webapps` submodule).
pub fn webserver_dir() -> Result<PathBuf> {
    Ok(project_guss()?.join("webserver"))
}

// --- config paths: `~/.config/guss/`, never inside a checkout ---

/// The directory the files below live in. Re-exported from [`crate::config`] so
/// callers have one place to look for paths.
pub fn config_dir() -> Result<PathBuf> {
    config::config_dir()
}

/// The legacy in-repo config directory, kept only so `setup init` can migrate
/// away from it. Nothing else should read this.
pub fn legacy_config_dir() -> Result<PathBuf> {
    Ok(project_guss()?.join("-config"))
}

/// Files migrated out of [`legacy_config_dir`], in the order reported to the user.
pub const CONFIG_FILES: &[&str] = &["mykey.txt", "db.toml", "device.key", "token.toml", "stacks.toml"];

/// `~/.config/guss/mykey.txt` — holds the ssh key name.
pub fn mykey_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("mykey.txt"))
}

/// `~/.config/guss/db.toml` — Postgres connection settings.
pub fn db_config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("db.toml"))
}

/// `~/.config/guss/device.key` — this machine's login signing key (0600).
pub fn device_key_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("device.key"))
}

/// `~/.config/guss/token.toml` — cached signed session token from the login
/// server (0600).
pub fn token_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("token.toml"))
}

/// `~/.config/guss/release.key` — the release signing key, on a publisher's
/// machine only (0600).
pub fn release_key_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("release.key"))
}

/// `~/.config/guss/stacks.toml` — the local docker stack registry read by `stack`.
pub fn stacks_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("stacks.toml"))
}

/// `~/project-guss/tools/vn-network-launch` — the AWS network CFN template repo.
pub fn network_launch_dir() -> Result<PathBuf> {
    Ok(tools_dir()?.join("vn-network-launch"))
}

/// The generated API docs / connections file that `docs search` reads.
/// `~/project-guss/applications/library/Connections/vapi-docs.md`
pub fn connections_doc() -> Result<PathBuf> {
    Ok(applications_dir()?.join("library").join("Connections").join("vapi-docs.md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway `$HOME` stand-in. `set_var` is `unsafe` in edition 2024 and
    /// would race the other tests in this binary, so the probe is tested through
    /// `resolve_root` with an explicit home instead of the real environment.
    fn fake_home(label: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        let dir = std::env::temp_dir().join(format!("guss_paths_{label}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Create `<home>/<rel>` and, when `marked`, the `tools/Cargo.toml` marker.
    fn candidate(home: &Path, rel: &str, marked: bool) {
        let dir = home.join(rel);
        std::fs::create_dir_all(&dir).unwrap();
        if marked {
            std::fs::create_dir_all(dir.join("tools")).unwrap();
            std::fs::write(dir.join(ROOT_MARKER), "[workspace]\n").unwrap();
        }
    }

    #[test]
    fn probe_prefers_the_first_marked_candidate() {
        let home = fake_home("both");
        candidate(&home, "project-guss", true);
        candidate(&home, "guss/project-guss", true);

        assert_eq!(resolve_root(&home), Some(home.join("project-guss")));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn probe_skips_an_unmarked_candidate() {
        // The real failure this fixes: ~/project-guss exists holding only
        // -config/, while the actual checkout is ~/guss/project-guss.
        let home = fake_home("stale_first");
        candidate(&home, "project-guss", false);
        std::fs::create_dir_all(home.join("project-guss").join("-config")).unwrap();
        candidate(&home, "guss/project-guss", true);

        assert_eq!(resolve_root(&home), Some(home.join("guss/project-guss")));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn probe_returns_none_when_nothing_is_marked() {
        let home = fake_home("none");
        candidate(&home, "project-guss", false);

        assert_eq!(resolve_root(&home), None);

        let _ = std::fs::remove_dir_all(&home);
    }
}
