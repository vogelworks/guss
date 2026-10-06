//! Canonical filesystem paths for the VHP layout.
//!
//! Hard-coded `/home/$USER/project-vhp/...` and `~/project-vapi/...` strings were
//! scattered across every shell script; they live here now. The monorepo root
//! itself is resolved by [`project_vhp`] rather than hard-coded, so a checkout
//! that is not at `~/project-vhp` still works.
//!
//! Two distinct kinds of path live here, and the difference matters:
//!
//! * **repo paths** ([`tools_dir`], [`network_dir`], [`applications_dir`], …) hang
//!   off the checkout, wherever that is;
//! * **config paths** ([`mykey_file`], [`token_file`], [`stacks_file`], …) hang off
//!   `~/.config/vhp` and are deliberately *outside* any checkout, so they survive
//!   `git clean`, are shared between checkouts, and do not depend on where the
//!   code happens to live.

use crate::config;
use crate::error::{Result, anyhow};
use std::path::{Path, PathBuf};

/// Env var that pins the monorepo root, bypassing the probe in [`project_vhp`].
pub const ROOT_ENV: &str = "VHP_ROOT";

/// Checkout locations tried, in order, when `VHP_ROOT` is unset.
const ROOT_CANDIDATES: &[&str] = &["project-vhp", "vhp/project-vhp"];

/// The marker that identifies a real checkout rather than a leftover directory
/// (`~/project-vhp` can exist holding only `-config/`).
const ROOT_MARKER: &str = "tools/Cargo.toml";

/// The user's home directory (`$HOME`).
pub fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME is not set"))
}

/// The monorepo root, in precedence order:
///
/// 1. `$VHP_ROOT` — the one-off escape hatch;
/// 2. `root` in `~/.config/vhp/config.toml` — the normal mechanism, written by
///    `setup init`, which is what lets each teammate keep their checkout wherever
///    they like;
/// 3. the [`ROOT_CANDIDATES`] probe — last resort, so a machine with no config
///    still works. The [`ROOT_MARKER`] check means a stale `~/project-vhp` holding
///    only `-config/` does not shadow a real `~/vhp/project-vhp`.
pub fn project_vhp() -> Result<PathBuf> {
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
            "no project-vhp checkout found (tried {tried}); run `vhp setup init`, \
             or set {ROOT_ENV} to its path"
        )
    })
}

/// First candidate under `home` that contains [`ROOT_MARKER`]. Split out so the
/// probe order is testable without touching `$HOME`.
fn resolve_root(home: &Path) -> Option<PathBuf> {
    ROOT_CANDIDATES
        .iter()
        .map(|c| home.join(c))
        .find(|dir| dir.join(ROOT_MARKER).exists())
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

/// `~/project-vhp/tools` — the Rust workspace root (where `setup` runs cargo).
pub fn tools_dir() -> Result<PathBuf> {
    Ok(project_vhp()?.join("tools"))
}

/// `~/project-vhp/network` (the `vn-network-services` submodule).
pub fn network_dir() -> Result<PathBuf> {
    Ok(project_vhp()?.join("network"))
}

/// `~/project-vhp/applications` (the `project-portal` submodule).
pub fn applications_dir() -> Result<PathBuf> {
    Ok(project_vhp()?.join("applications"))
}

/// `~/project-vhp/webserver` (the `vs-webapps` submodule).
pub fn webserver_dir() -> Result<PathBuf> {
    Ok(project_vhp()?.join("webserver"))
}

// --- config paths: `~/.config/vhp/`, never inside a checkout ---

/// The directory the files below live in. Re-exported from [`crate::config`] so
/// callers have one place to look for paths.
pub fn config_dir() -> Result<PathBuf> {
    config::config_dir()
}

/// The legacy in-repo config directory, kept only so `setup init` can migrate
/// away from it. Nothing else should read this.
pub fn legacy_config_dir() -> Result<PathBuf> {
    Ok(project_vhp()?.join("-config"))
}

/// Files migrated out of [`legacy_config_dir`], in the order reported to the user.
pub const CONFIG_FILES: &[&str] = &[
    "mykey.txt",
    "db.toml",
    "device.key",
    "token.toml",
    "stacks.toml",
];

/// `~/.config/vhp/mykey.txt` — holds the ssh key name.
pub fn mykey_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("mykey.txt"))
}

/// `~/.config/vhp/db.toml` — Postgres connection settings.
pub fn db_config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("db.toml"))
}

/// `~/.config/vhp/device.key` — this machine's login signing key (0600).
pub fn device_key_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("device.key"))
}

/// `~/.config/vhp/token.toml` — cached signed session token from the login
/// server (0600).
pub fn token_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("token.toml"))
}

/// `~/.config/vhp/release.key` — the release signing key, on a publisher's
/// machine only (0600).
pub fn release_key_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("release.key"))
}

/// `~/.config/vhp/stacks.toml` — the local docker stack registry read by `stack`.
pub fn stacks_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("stacks.toml"))
}

/// `~/project-vhp/tools/vn-network-launch` — the AWS network CFN template repo.
pub fn network_launch_dir() -> Result<PathBuf> {
    Ok(tools_dir()?.join("vn-network-launch"))
}

/// The generated API docs / connections file that `docs search` reads.
/// `~/project-vhp/applications/library/Connections/vapi-docs.md`
pub fn connections_doc() -> Result<PathBuf> {
    Ok(applications_dir()?
        .join("library")
        .join("Connections")
        .join("vapi-docs.md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway `$HOME` stand-in. `set_var` is `unsafe` in edition 2024 and
    /// would race the other tests in this binary, so the probe is tested through
    /// `resolve_root` with an explicit home instead of the real environment.
    fn fake_home(label: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!("vhp_paths_{label}_{nanos}"));
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
        candidate(&home, "project-vhp", true);
        candidate(&home, "vhp/project-vhp", true);

        assert_eq!(resolve_root(&home), Some(home.join("project-vhp")));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn probe_skips_an_unmarked_candidate() {
        // The real failure this fixes: ~/project-vhp exists holding only
        // -config/, while the actual checkout is ~/vhp/project-vhp.
        let home = fake_home("stale_first");
        candidate(&home, "project-vhp", false);
        std::fs::create_dir_all(home.join("project-vhp").join("-config")).unwrap();
        candidate(&home, "vhp/project-vhp", true);

        assert_eq!(resolve_root(&home), Some(home.join("vhp/project-vhp")));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn probe_returns_none_when_nothing_is_marked() {
        let home = fake_home("none");
        candidate(&home, "project-vhp", false);

        assert_eq!(resolve_root(&home), None);

        let _ = std::fs::remove_dir_all(&home);
    }
}
