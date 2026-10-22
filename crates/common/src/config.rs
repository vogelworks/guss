//! Persistent settings for the tools, stored **outside** any checkout.
//!
//! Config used to live at `<repo>/-config/`, which made the config location a
//! function of the code location — that is why every path in the tool broke on a
//! machine whose checkout was not at `~/project-guss`. It now lives at
//! `${XDG_CONFIG_HOME:-~/.config}/guss/`, and the *repo* path is recorded in it
//! rather than the other way round.
//!
//! Everything here depends only on [`paths::home`]. It must never reach for
//! [`paths::project_guss`], which reads these settings — that would recurse.

use crate::error::{Result, anyhow};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Default ssh key name when `mykey.txt` is missing or empty.
pub const DEFAULT_KEY: &str = "devteam";

/// Filename of the settings document inside [`config_dir`].
pub const CONFIG_FILE: &str = "config.toml";

/// `${XDG_CONFIG_HOME:-~/.config}/guss` — where all tool state lives.
pub fn config_dir() -> Result<PathBuf> {
    Ok(config_dir_from(std::env::var_os("XDG_CONFIG_HOME").as_deref(), &paths::home()?))
}

/// Split out so the XDG rule is testable without mutating the environment
/// (`set_var` is `unsafe` in edition 2024 and would race sibling tests).
fn config_dir_from(xdg: Option<&OsStr>, home: &Path) -> PathBuf {
    match xdg.filter(|v| !v.is_empty()) {
        Some(base) => Path::new(base).join("guss"),
        None => home.join(".config").join("guss"),
    }
}

/// The settings document itself.
pub fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join(CONFIG_FILE))
}

/// Central login server settings. **Absent means the permission gate is off** —
/// see `auth::enforce`. Both fields were compile-time constants before, which is
/// why the tool could not be pointed at a different server without a re-release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSettings {
    /// Base URL of the login server, e.g. `http://dev.gussportal.office`.
    pub login_host: String,
    /// The login server's ed25519 public key, base64. Tokens are verified
    /// against this.
    pub server_key: String,
}

/// Where signed release tarballs are published and fetched from. Absent means
/// `setup update` / `setup publish` are unavailable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseSettings {
    /// HTTP base for downloads, e.g. `http://dev.gussportal.office/tools`.
    pub base_url: String,
    /// ssh destination `publish` uploads to, e.g. `devteam@dev.gussportal.office`.
    pub ssh_host: String,
    /// Directory on that host holding the per-channel subdirectories.
    pub remote_dir: String,
    /// Release-signing public key (base64), overriding the compiled-in default.
    /// Present so the key can be rotated without shipping a new binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

/// Deployment mode (dev/prod/test) — used to prefix service names for SSM and routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Dev,
    Prod,
    Test,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Mode::Dev => "dev",
            Mode::Prod => "prod",
            Mode::Test => "test",
        })
    }
}

impl std::str::FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dev" => Ok(Mode::Dev),
            "prod" => Ok(Mode::Prod),
            "test" => Ok(Mode::Test),
            _ => Err(format!("unknown mode: {}", s)),
        }
    }
}

impl Mode {
    /// Docker image tag for this mode (test mode uses dev tag).
    pub fn docker_tag(&self) -> &'static str {
        match self {
            Mode::Dev => "dev",
            Mode::Prod => "prod",
            Mode::Test => "dev",
        }
    }

    /// Deployment mode (test mode deploys as dev).
    pub fn for_deployment(&self) -> Mode {
        match self {
            Mode::Test => Mode::Dev,
            other => *other,
        }
    }
}

/// Build configuration: apps to build, source, and destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    pub name: String,
    pub root_src: String,
    pub root_dest: String,
    pub deploy: Option<BuildDeploy>,
    pub apps: Vec<BuildApp>,
    /// Whether to sync version from source to destination repo (default: true)
    #[serde(default = "default_sync_version")]
    pub sync_version: bool,
}

fn default_sync_version() -> bool {
    true
}

/// Deployment configuration for a build.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BuildDeploy {
    Web { cluster: String, container: String },
    Desktop { command: String },
}

/// Single app in a build configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildApp {
    pub name: String,
    pub npm_script: String,
    pub src_dir: String,
    pub dest_path: String,
}

/// Documentation generation configuration for a project.
/// Root folder contains search-able markdown. Optional builds array defines automated generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocConfig {
    pub name: String,
    pub root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builds: Option<Vec<DocBuild>>,
}

/// A build configuration for automated doc generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocBuild {
    pub r#type: String,
    /// Root folder for this build (relative to config.toml root). Overrides DocConfig root if set.
    #[serde(default)]
    pub root: Option<PathBuf>,
    /// Output file path (relative to root) for this build. Defaults to "{type}.md".
    #[serde(default)]
    pub output: Option<PathBuf>,
    #[serde(default = "default_servers")]
    pub servers: String,
    #[serde(default = "default_scan")]
    pub scan: String,
    #[serde(default)]
    pub skip_dirs: Vec<String>,
    #[serde(default = "default_file")]
    pub file: String,
    #[serde(default)]
    pub config: DocConfigPattern,
    #[serde(default)]
    pub docs: DocDocsPattern,
}

fn default_servers() -> String {
    "vapi-*".to_string()
}

fn default_scan() -> String {
    "controllers".to_string()
}

fn default_file() -> String {
    "*Controller.js".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocConfigPattern {
    #[serde(default = "default_config_file")]
    pub file: String,
    #[serde(default = "default_routes_start")]
    pub routes_start: String,
    #[serde(default = "default_route_name")]
    pub route_name: String,
}

fn default_config_file() -> String {
    "{stem}config.js".to_string()
}

fn default_routes_start() -> String {
    "routes:".to_string()
}

fn default_route_name() -> String {
    "name:".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocDocsPattern {
    #[serde(default = "default_block_start")]
    pub block_start: String,
    #[serde(default = "default_block_end")]
    pub block_end: String,
    #[serde(default = "default_method_pattern")]
    pub method_pattern: String,
    #[serde(default = "default_fallback")]
    pub fallback: String,
}

fn default_block_start() -> String {
    "/**".to_string()
}

fn default_block_end() -> String {
    "*/".to_string()
}

fn default_method_pattern() -> String {
    "{route} = async".to_string()
}

fn default_fallback() -> String {
    "@note — no docs".to_string()
}

/// The whole settings document. Every field optional: a machine with no config
/// file still works, falling back to the path probe in [`paths::project_guss`].
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    /// The `project-guss` checkout this machine uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    /// The vapi services tree, if it is not `~/project-vapi`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vapi_root: Option<PathBuf>,
    /// Current deployment mode (dev/prod/test).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// SSH key name for docker builds (e.g. `id_ed25519`, `id_rsa`; defaults to `id_ed25519`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_key: Option<String>,
    /// Repo paths (relative to home dir) whose branches should follow the deployment mode.
    /// When mode changes, branches switch to match (dev/prod modes, test→dev).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracked_repos: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseSettings>,
    /// Cluster configuration file paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<String>,
    /// App configuration file paths (build, sources, destinations).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub app_configs: Vec<String>,
    /// Documentation configuration file paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc_configs: Vec<String>,
}

impl Settings {
    /// Load the settings, treating a missing file as all-defaults. Deliberately
    /// uncached: `setup init` writes the file and then keeps running, so a
    /// process-wide cache would serve stale values mid-command.
    pub fn load() -> Result<Self> {
        Self::load_from(&config_file()?)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|e| anyhow!("{}: {e}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = config_file()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Get the current deployment mode, returning an error if not configured.
    pub fn get_mode(&self) -> Result<Mode> {
        self.mode
            .ok_or_else(|| anyhow!("mode not configured. Run 'guss net init' to set it up or 'guss net mode <dev|prod|test>' to change it."))
    }

    /// Set the deployment mode and save to disk.
    pub fn set_mode(&mut self, mode: Mode) -> Result<()> {
        self.mode = Some(mode);
        self.save()
    }

    /// Get the SSH key name for docker builds (from config or default to `id_ed25519`).
    pub fn get_ssh_key(&self) -> String {
        self.ssh_key.clone().unwrap_or_else(|| "id_ed25519".to_string())
    }

    /// Get default path to an app repo (applications, webserver).
    /// Used for mode branch switching; the actual paths come from app_configs.
    pub fn get_app_repo_path(&self, name: &str) -> Result<PathBuf> {
        let root = self.root.clone().unwrap_or(crate::paths::project_guss()?);
        match name {
            "applications" | "webserver" => Ok(root.join(name)),
            _ => Err(anyhow!("unknown app repo: {name}")),
        }
    }

    /// Load an app config by name from configured app_configs paths.
    pub fn get_app_config(&self, name: &str) -> Result<BuildConfig> {
        for config_path in &self.app_configs {
            let root = self.root.clone().unwrap_or(crate::paths::project_guss()?);
            let path = if Path::new(config_path).is_absolute() {
                PathBuf::from(config_path)
            } else {
                root.join(config_path)
            };
            if !path.exists() {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            let config: BuildConfig = serde_json::from_str(&text).map_err(|e| anyhow!("{}: {e}", path.display()))?;
            if config.name == name {
                return Ok(config);
            }
        }
        Err(anyhow!("build config '{}' not found", name))
    }

    /// Load a doc config by name from configured doc_configs paths.
    pub fn get_doc_config(&self, name: &str) -> Result<DocConfig> {
        for config_path in &self.doc_configs {
            let root = self.root.clone().unwrap_or(crate::paths::project_guss()?);
            let path = if Path::new(config_path).is_absolute() {
                PathBuf::from(config_path)
            } else {
                root.join(config_path)
            };
            if !path.exists() {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            let config: DocConfig = serde_json::from_str(&text).map_err(|e| anyhow!("{}: {e}", path.display()))?;
            if config.name == name {
                return Ok(config);
            }
        }
        Err(anyhow!("doc config '{}' not found", name))
    }

    /// Load all doc configs from configured doc_configs paths.
    pub fn doc_configs(&self) -> Result<Vec<DocConfig>> {
        let mut configs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for config_path in &self.doc_configs {
            let root = self.root.clone().unwrap_or(crate::paths::project_guss()?);
            let path = if Path::new(config_path).is_absolute() {
                PathBuf::from(config_path)
            } else {
                root.join(config_path)
            };
            if !path.exists() {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            let config: DocConfig = serde_json::from_str(&text).map_err(|e| anyhow!("{}: {e}", path.display()))?;
            if seen.insert(config.name.clone()) {
                configs.push(config);
            }
        }
        Ok(configs)
    }
}

/// Return the configured ssh key name, falling back to [`DEFAULT_KEY`].
pub fn mykey() -> Result<String> {
    let path = paths::mykey_file()?;
    let value = fs::read_to_string(&path).unwrap_or_default();
    let value = value.trim();
    Ok(if value.is_empty() { DEFAULT_KEY.to_string() } else { value.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        let dir = std::env::temp_dir().join(format!("guss_settings_{label}_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn config_dir_defaults_under_home() {
        assert_eq!(config_dir_from(None, Path::new("/home/alice")), PathBuf::from("/home/alice/.config/guss"));
    }

    #[test]
    fn config_dir_honors_xdg() {
        assert_eq!(
            config_dir_from(Some(OsStr::new("/custom/cfg")), Path::new("/home/alice")),
            PathBuf::from("/custom/cfg/guss")
        );
    }

    #[test]
    fn empty_xdg_falls_back_to_home() {
        // An exported-but-empty XDG_CONFIG_HOME is common and must not produce
        // a config path rooted at "/guss".
        assert_eq!(
            config_dir_from(Some(OsStr::new("")), Path::new("/home/alice")),
            PathBuf::from("/home/alice/.config/guss")
        );
    }

    #[test]
    fn missing_file_loads_as_defaults() {
        let dir = temp_dir("missing");
        let settings = Settings::load_from(&dir.join("nope.toml")).unwrap();

        assert!(settings.root.is_none());
        assert!(settings.auth.is_none());
        assert!(settings.release.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_document_round_trips() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(CONFIG_FILE);
        fs::write(
            &path,
            r#"
root = "/home/alice/work/project-guss"
vapi_root = "/home/alice/work/project-guss/servers"
mode = "dev"
tracked_repos = ["~/project-guss/servers", "~/project-guss/network"]

[auth]
login_host = "http://dev.gussportal.office"
server_key = "AAAA"

[release]
base_url = "http://dev.gussportal.office/tools"
ssh_host = "devteam@dev.gussportal.office"
remote_dir = "/srv/tools"
"#,
        )
        .unwrap();

        let settings = Settings::load_from(&path).unwrap();
        assert_eq!(settings.root, Some(PathBuf::from("/home/alice/work/project-guss")));
        assert_eq!(settings.vapi_root, Some(PathBuf::from("/home/alice/work/project-guss/servers")));
        assert_eq!(settings.mode, Some(Mode::Dev));
        assert_eq!(settings.tracked_repos.len(), 2);
        assert_eq!(settings.auth.as_ref().unwrap().server_key, "AAAA");
        let release = settings.release.as_ref().unwrap();
        assert_eq!(release.remote_dir, "/srv/tools");
        // Omitted in the document, so rotation is simply not configured.
        assert!(release.public_key.is_none());

        // Re-serializing must not invent the optional fields back.
        let text = toml::to_string_pretty(&settings).unwrap();
        assert!(!text.contains("public_key"), "got: {text}");
        let reparsed: Settings = toml::from_str(&text).unwrap();
        assert_eq!(reparsed.root, settings.root);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auth_section_alone_is_enough() {
        // The gate keys off this section's presence, so it must parse with no
        // root/release alongside it.
        let settings: Settings = toml::from_str("[auth]\nlogin_host = \"http://h\"\nserver_key = \"k\"\n").unwrap();
        assert!(settings.auth.is_some());
        assert!(settings.root.is_none());
    }

    #[test]
    fn invalid_toml_names_the_file() {
        let dir = temp_dir("bad");
        let path = dir.join(CONFIG_FILE);
        fs::write(&path, "not [ valid toml").unwrap();

        let err = Settings::load_from(&path).unwrap_err().to_string();
        assert!(err.contains(CONFIG_FILE), "got: {err}");

        let _ = fs::remove_dir_all(&dir);
    }
}
