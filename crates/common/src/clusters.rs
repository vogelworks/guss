//! Cluster and container configuration, loaded from JSON files listed in config.toml.
//! Allows bench command to reuse and organize clusters and their containers as data.

use crate::error::{Result, anyhow};
use crate::{config, paths};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EcsCluster {
    /// ECS cluster name (supports {mode} template, defaults to guss-cluster-{mode}).
    #[serde(default)]
    pub cluster: String,
    /// AWS region for ECR/ECS operations (falls back to ssm::get_region()).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// AWS account ID for ECR (falls back to aws sts get-caller-identity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EcsContainer {
    /// Dockerfile path relative to source (defaults to Dockerfile).
    #[serde(default)]
    pub dockerfile: String,
    /// Docker build arguments (e.g., ["BIN=gateway"]).
    #[serde(default)]
    pub build_args: Vec<String>,
    /// ECS service name (defaults to container name/alias).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// ECR repository name (defaults to container name/alias).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecr_repo: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClusterConfig {
    pub name: String,
    /// Docker compose file (optional for ecs clusters).
    #[serde(default)]
    pub compose: Option<PathBuf>,
    /// Environment type: "local", "net", or "ecs".
    #[serde(default = "default_environment")]
    pub environment: String,
    /// ECS-specific configuration (only for environment: "ecs").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecs: Option<EcsCluster>,
    #[serde(default)]
    pub containers: BTreeMap<String, ContainerConfig>,
}

fn default_environment() -> String {
    "net".to_string()
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContainerConfig {
    /// Docker-compose service name (if different from the config key).
    #[serde(default)]
    pub alias: Option<String>,
    /// Build source path (explicit, independent of name/alias).
    #[serde(default)]
    pub source: Option<PathBuf>,
    /// Config file path to inject during build (passed as CONFIG_PATH build arg).
    #[serde(default)]
    pub config: Option<PathBuf>,
    #[serde(default = "default_bulk")]
    pub bulk: bool,
    /// Git repo root this container depends on (for branch switching during deploy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Skip branch switching checks for this container.
    #[serde(default, skip_serializing_if = "<&bool>::clone")]
    pub no_branch_switch: bool,
    /// Lock this container to a specific deployment mode (dev/prod/test).
    /// Only applies in environment="net". Prevents accidental deployment to wrong mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Docker build context path relative to the Dockerfile directory (default: ".").
    #[serde(default = "default_build_context")]
    pub build_context: String,
    /// ECS-specific configuration (only for environment: "ecs").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecs: Option<EcsContainer>,
}

fn default_build_context() -> String {
    ".".to_string()
}

fn default_bulk() -> bool {
    true
}

#[derive(Clone)]
pub struct ResolvedContainer {
    pub name: String,
    pub source: PathBuf,
    pub config: Option<PathBuf>,
    pub bulk: bool,
    pub build_context: String,
    pub ecs: Option<EcsContainer>,
}

impl ClusterConfig {
    /// Path to this cluster's docker-compose.yml, resolved against root.
    /// Returns None if compose is not configured (e.g., for ecs clusters).
    pub fn compose_path(&self, root: &Path) -> Option<PathBuf> {
        self.compose.as_ref().map(|p| root.join(p))
    }

    /// Resolve a container name to its build source path.
    /// Uses the alias if set, otherwise the config key.
    /// Source must be explicitly provided. Config path is resolved relative to root.
    pub fn resolve(&self, name: &str, root: &Path, _vapi_root: &Path) -> Result<ResolvedContainer> {
        let cfg = self.containers.get(name).ok_or_else(|| {
            anyhow!(
                "cluster '{}' has no container '{}' (has: {})",
                self.name,
                name,
                self.containers.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        let source = cfg
            .source
            .as_ref()
            .ok_or_else(|| anyhow!("container '{}' in cluster '{}' requires explicit 'source' field", name, self.name))?;
        let container_name = cfg.alias.clone().unwrap_or_else(|| name.to_string());
        let config = cfg.config.as_ref().map(|p| root.join(p));
        Ok(ResolvedContainer {
            name: container_name,
            source: root.join(source),
            config,
            bulk: cfg.bulk,
            build_context: cfg.build_context.clone(),
            ecs: cfg.ecs.clone(),
        })
    }

    /// Iterator over container names included in bulk operations.
    pub fn bulk_containers(&self) -> impl Iterator<Item = &str> {
        self.containers.iter().filter(|(_, c)| c.bulk).map(|(n, _)| n.as_str())
    }

    /// Load a cluster from a JSON file.
    fn load_file(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|e| anyhow!("reading cluster config {}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| anyhow!("{}: {e}", path.display()))
    }
}

/// Load every cluster listed under [clusters] in config.toml.
pub fn load() -> Result<Vec<ClusterConfig>> {
    let settings = config::Settings::load()?;
    let root = paths::project_guss()?;

    if settings.clusters.is_empty() {
        return Err(anyhow!("no clusters configured under [clusters] in config.toml"));
    }

    settings.clusters.iter().map(|rel| ClusterConfig::load_file(&root.join(rel))).collect()
}

/// Find a cluster by name in a loaded set, returning an error with known names.
pub fn find<'a>(clusters: &'a [ClusterConfig], name: &str) -> Result<&'a ClusterConfig> {
    clusters.iter().find(|c| c.name == name).ok_or_else(|| {
        anyhow!(
            "unknown cluster '{name}' (known: {})",
            clusters.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        let dir = std::env::temp_dir().join(format!("guss_clusters_test_{label}_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_explicit_source_container() {
        let dir = temp_dir("explicit");
        let cluster_json = dir.join("cluster.json");
        fs::write(
            &cluster_json,
            r#"{
  "name": "test",
  "compose": "test/compose.yml",
  "containers": {
    "pluginrp": { "source": "servers/vapi-pluginrp" }
  }
}"#,
        )
        .unwrap();

        let cluster = ClusterConfig::load_file(&cluster_json).unwrap();
        let resolved = cluster.resolve("pluginrp", &dir, &dir.join("vapi_root")).unwrap();

        assert_eq!(resolved.name, "pluginrp");
        assert_eq!(resolved.source, dir.join("servers/vapi-pluginrp"));
        assert!(resolved.bulk);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_alias_override() {
        let dir = temp_dir("alias");
        let cluster_json = dir.join("cluster.json");
        fs::write(
            &cluster_json,
            r#"{
  "name": "test",
  "compose": "test/compose.yml",
  "containers": {
    "core": { "alias": "admin", "source": "servers/vapi-admin" }
  }
}"#,
        )
        .unwrap();

        let cluster = ClusterConfig::load_file(&cluster_json).unwrap();
        let resolved = cluster.resolve("core", &dir, &dir.join("vapi_root")).unwrap();

        assert_eq!(resolved.name, "admin");
        assert_eq!(resolved.source, dir.join("servers/vapi-admin"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_source_override() {
        let dir = temp_dir("source");
        let cluster_json = dir.join("cluster.json");
        fs::write(
            &cluster_json,
            r#"{
  "name": "test",
  "compose": "test/compose.yml",
  "containers": {
    "core-lb": { "source": "network/test-cluster/core-lb", "bulk": false }
  }
}"#,
        )
        .unwrap();

        let cluster = ClusterConfig::load_file(&cluster_json).unwrap();
        let resolved = cluster.resolve("core-lb", &dir, &dir.join("vapi_root")).unwrap();

        assert_eq!(resolved.name, "core-lb");
        assert_eq!(resolved.source, dir.join("network/test-cluster/core-lb"));
        assert!(!resolved.bulk);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bulk_containers_filters_correctly() {
        let dir = temp_dir("bulk");
        let cluster_json = dir.join("cluster.json");
        fs::write(
            &cluster_json,
            r#"{
  "name": "test",
  "compose": "test/compose.yml",
  "containers": {
    "app": { "bulk": true },
    "service": { "bulk": true },
    "lb": { "bulk": false }
  }
}"#,
        )
        .unwrap();

        let cluster = ClusterConfig::load_file(&cluster_json).unwrap();
        let bulk: Vec<&str> = cluster.bulk_containers().collect();

        assert_eq!(bulk.len(), 2);
        assert!(bulk.contains(&"app"));
        assert!(bulk.contains(&"service"));
        assert!(!bulk.contains(&"lb"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_cluster_by_name() {
        let dir = temp_dir("find");
        let c1 = dir.join("c1.json");
        let c2 = dir.join("c2.json");

        fs::write(&c1, r#"{ "name": "core", "compose": "test.yml", "containers": {} }"#).unwrap();
        fs::write(&c2, r#"{ "name": "portal", "compose": "test.yml", "containers": {} }"#).unwrap();

        let clusters = vec![ClusterConfig::load_file(&c1).unwrap(), ClusterConfig::load_file(&c2).unwrap()];

        let found = find(&clusters, "portal").unwrap();
        assert_eq!(found.name, "portal");

        let not_found = find(&clusters, "nope");
        assert!(not_found.is_err());
        assert!(not_found.unwrap_err().to_string().contains("known: core, portal"));

        fs::remove_dir_all(&dir).unwrap();
    }
}
