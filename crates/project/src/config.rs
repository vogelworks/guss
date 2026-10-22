use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use guss_common::{Result, error::anyhow};

const CONFIG_FILE: &str = ".gitflow";

#[derive(Debug, Serialize, Deserialize)]
pub struct BranchConfig {
    pub main: String,
    pub develop: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PrefixConfig {
    pub feature: String,
    pub release: String,
    pub hotfix: String,
    pub tag: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PrConfig {
    /// Whether to open a PR automatically after publish
    pub enabled: bool,
}

/// A hook run after a lifecycle action completes.
///
/// `event` is a dot-namespaced action name (`feature.publish`,
/// `release.finish`, `hotfix.start`, `init`, ...) matching the command that
/// just ran. `branch` is an optional glob (`*` wildcard) narrowing which
/// branch the hook applies to; omitted means any branch for that event.
#[derive(Debug, Serialize, Deserialize)]
pub struct HookConfig {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub run: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub branch: BranchConfig,
    pub prefix: PrefixConfig,
    pub pr: PrConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            branch: BranchConfig {
                main: "main".to_string(),
                develop: "develop".to_string(),
            },
            prefix: PrefixConfig {
                feature: "feature/".to_string(),
                release: "release/".to_string(),
                hotfix: "hotfix/".to_string(),
                tag: "".to_string(),
            },
            pr: PrConfig { enabled: true },
            hooks: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        if !Path::new(CONFIG_FILE).exists() {
            return Err(anyhow!(
                "not a project repository (no .gitflow file found). Run `guss project init` first."
            ));
        }
        let contents = fs::read_to_string(CONFIG_FILE)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let contents = toml::to_string_pretty(self)?;
        fs::write(CONFIG_FILE, contents)?;
        Ok(())
    }

    pub fn feature_branch(&self, name: &str) -> String {
        format!("{}{}", self.prefix.feature, name)
    }

    pub fn release_branch(&self, version: &str) -> String {
        format!("{}{}", self.prefix.release, version)
    }

    pub fn release_tag(&self, version: &str) -> String {
        format!("{}{}", self.prefix.tag, version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Pure unit tests (no I/O) ---

    #[test]
    fn default_config_values() {
        let c = Config::default();
        assert_eq!(c.branch.main, "main");
        assert_eq!(c.branch.develop, "develop");
        assert_eq!(c.prefix.feature, "feature/");
        assert_eq!(c.prefix.release, "release/");
        assert_eq!(c.prefix.hotfix, "hotfix/");
        assert_eq!(c.prefix.tag, "");
        assert!(c.pr.enabled);
    }

    #[test]
    fn feature_branch_name() {
        let c = Config::default();
        assert_eq!(c.feature_branch("my-feature"), "feature/my-feature");
    }

    #[test]
    fn release_branch_name() {
        let c = Config::default();
        assert_eq!(c.release_branch("1.0.0"), "release/1.0.0");
    }

    #[test]
    fn release_tag_no_prefix() {
        let c = Config::default();
        assert_eq!(c.release_tag("2.3.4"), "2.3.4");
    }

    #[test]
    fn release_tag_with_v_prefix() {
        let mut c = Config::default();
        c.prefix.tag = "v".to_string();
        assert_eq!(c.release_tag("2.3.4"), "v2.3.4");
    }

    // --- Filesystem tests (require serialized dir access via DIR_LOCK) ---

    fn make_empty_dir(label: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        let dir = std::env::temp_dir().join(format!("gust_cfg_{}_{}", label, nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_errors_without_gitflow() {
        let _lock = crate::test_helpers::DIR_LOCK.lock().unwrap();
        let dir = make_empty_dir("no_gitflow");
        let _orig = crate::test_helpers::OrigDir::save_and_switch(&dir);

        let err = Config::load().unwrap_err().to_string();
        assert!(err.contains("guss project init"), "expected mention of `guss project init`, got: {}", err);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_errors_on_invalid_toml() {
        let _lock = crate::test_helpers::DIR_LOCK.lock().unwrap();
        let dir = make_empty_dir("bad_toml");
        std::fs::write(dir.join(".gitflow"), "not [ valid toml").unwrap();
        let _orig = crate::test_helpers::OrigDir::save_and_switch(&dir);

        assert!(Config::load().is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let _lock = crate::test_helpers::DIR_LOCK.lock().unwrap();
        let dir = make_empty_dir("roundtrip");
        let _orig = crate::test_helpers::OrigDir::save_and_switch(&dir);

        let mut c = Config::default();
        c.branch.main = "trunk".to_string();
        c.prefix.tag = "v".to_string();
        c.pr.enabled = false;
        c.save().unwrap();

        let loaded = Config::load().unwrap();
        assert_eq!(loaded.branch.main, "trunk");
        assert_eq!(loaded.prefix.tag, "v");
        assert!(!loaded.pr.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
