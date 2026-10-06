//! Version parsing, comparison, and release tracking.

use crate::error::Result;
use anyhow::anyhow;
use chrono::Local;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

/// Parse `1.2.3` or `v1.2.3` into a comparable tuple. Returns `None` for
/// anything else, which callers treat as "cannot compare", not "older".
/// Rejects pre-release versions (e.g., `1.2.3-rc1`) by returning `None`.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim().trim_start_matches('v');
    // Reject pre-release versions: if there's a hyphen or plus, it's not a plain semver
    if text.contains('-') || text.contains('+') {
        return None;
    }
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether `candidate` is strictly newer than `current`. Unparseable versions are
/// never treated as newer — better to reject the input than to upgrade on a guess.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Read the `"version"` field from a package.json file.
pub fn pkg_version(dir: &Path) -> Result<String> {
    let pkg_file = dir.join("package.json");
    let content = fs::read_to_string(&pkg_file).map_err(|e| anyhow!("failed to read {}: {}", pkg_file.display(), e))?;
    let value: Value = serde_json::from_str(&content).map_err(|e| anyhow!("failed to parse {}: {}", pkg_file.display(), e))?;
    value
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("no 'version' field in {}", pkg_file.display()))
}

/// Write a version string to the `"version"` field in a package.json file.
/// Preserves 2-space indentation if the file already exists.
pub fn set_pkg_version(dir: &Path, version: &str) -> Result<()> {
    let pkg_file = dir.join("package.json");
    let content = fs::read_to_string(&pkg_file).map_err(|e| anyhow!("failed to read {}: {}", pkg_file.display(), e))?;
    let mut value: Value = serde_json::from_str(&content).map_err(|e| anyhow!("failed to parse {}: {}", pkg_file.display(), e))?;
    value["version"] = Value::String(version.to_string());
    let formatted = serde_json::to_string_pretty(&value).map_err(|e| anyhow!("failed to serialize {}: {}", pkg_file.display(), e))?;
    // serde_json::to_string_pretty uses 2-space indent by default
    fs::write(&pkg_file, formatted + "\n").map_err(|e| anyhow!("failed to write {}: {}", pkg_file.display(), e))
}

/// Check that `new_version` is strictly greater than the package.json version at `pkg_dir`
/// and the highest recorded version for the target in the versions table at `root`.
/// Returns an error message that names which gate failed.
pub fn check(root: &Path, target: &str, new_version: &str, pkg_dir: &Path) -> Result<()> {
    // Reject pre-release versions.
    if parse_version(new_version).is_none() {
        return Err(anyhow!(
            "version '{}' is invalid (must be X.Y.Z, with no pre-release suffix)",
            new_version
        ));
    }

    // Check against package.json
    let pkg_ver = pkg_version(pkg_dir)?;
    if !is_newer(new_version, &pkg_ver) {
        return Err(anyhow!(
            "version '{}' is not greater than {} ({})",
            new_version,
            pkg_ver,
            pkg_dir.join("package.json").display()
        ));
    }

    // Check against the table
    let versions_file = root.join("VERSIONS.json");
    if versions_file.exists() {
        if let Some(max_recorded) = max_version_for_target(&versions_file, target)? {
            if !is_newer(new_version, &max_recorded) {
                return Err(anyhow!(
                    "version '{}' is not greater than {} (highest recorded for '{}')",
                    new_version,
                    max_recorded,
                    target
                ));
            }
        }
    }

    Ok(())
}

/// Check that `version` is strictly greater than the highest recorded version for the target
/// in the versions table at `root`. Used when auto-detecting version from package.json.
/// Returns an error only if version <= recorded max. Allows version if no prior record exists.
pub fn check_table_only(root: &Path, target: &str, version: &str) -> Result<()> {
    // Reject pre-release versions.
    if parse_version(version).is_none() {
        return Err(anyhow!(
            "version '{}' is invalid (must be X.Y.Z, with no pre-release suffix)",
            version
        ));
    }

    // Check against the table only
    let versions_file = root.join("VERSIONS.json");
    if versions_file.exists() {
        if let Some(max_recorded) = max_version_for_target(&versions_file, target)? {
            if !is_newer(version, &max_recorded) {
                return Err(anyhow!(
                    "version '{}' is not greater than {} (highest recorded for '{}')",
                    version,
                    max_recorded,
                    target
                ));
            }
        }
    }

    Ok(())
}

/// Append an entry to the VERSIONS.json file. Creates the file if it doesn't exist.
/// Does nothing if `dry_run` is true.
pub fn record(root: &Path, target: &str, version: &str, mode: &str, dry_run: bool) -> Result<()> {
    if dry_run {
        return Ok(());
    }

    let versions_file = root.join("VERSIONS.json");
    let timestamp = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();

    let mut data = if versions_file.exists() {
        let content = fs::read_to_string(&versions_file)
            .map_err(|e| anyhow!("failed to read {}: {}", versions_file.display(), e))?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow!("failed to parse {}: {}", versions_file.display(), e))?
    } else {
        json!({ "entries": [], "deployments": [] })
    };

    if let Some(entries) = data.get_mut("entries").and_then(|e| e.as_array_mut()) {
        entries.push(json!({
            "target": target,
            "version": version,
            "mode": mode,
            "date": timestamp
        }));
    }

    let formatted = serde_json::to_string_pretty(&data)
        .map_err(|e| anyhow!("failed to serialize {}: {}", versions_file.display(), e))?;
    fs::write(&versions_file, formatted + "\n")
        .map_err(|e| anyhow!("failed to write {}: {}", versions_file.display(), e))
}

/// Log a deployment to the VERSIONS.json file. Creates the file if it doesn't exist.
/// Does nothing if `dry_run` is true. `version` can be None for unversioned containers.
pub fn log_deployment(root: &Path, target: &str, version: Option<&str>, mode: &str, dry_run: bool) -> Result<()> {
    if dry_run {
        return Ok(());
    }

    let versions_file = root.join("VERSIONS.json");
    let timestamp = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();

    let mut data = if versions_file.exists() {
        let content = fs::read_to_string(&versions_file)
            .map_err(|e| anyhow!("failed to read {}: {}", versions_file.display(), e))?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow!("failed to parse {}: {}", versions_file.display(), e))?
    } else {
        json!({ "entries": [], "deployments": [] })
    };

    // Ensure deployments array exists
    if !data.get("deployments").map_or(false, |d| d.is_array()) {
        data["deployments"] = json!([]);
    }

    if let Some(deployments) = data.get_mut("deployments").and_then(|d| d.as_array_mut()) {
        let entry = match version {
            Some(v) => json!({
                "target": target,
                "version": v,
                "mode": mode,
                "deployed_at": timestamp
            }),
            None => json!({
                "target": target,
                "mode": mode,
                "deployed_at": timestamp
            }),
        };
        deployments.push(entry);
    }

    let formatted = serde_json::to_string_pretty(&data)
        .map_err(|e| anyhow!("failed to serialize {}: {}", versions_file.display(), e))?;
    fs::write(&versions_file, formatted + "\n")
        .map_err(|e| anyhow!("failed to write {}: {}", versions_file.display(), e))
}

/// Get all versions and deployments for a build config from VERSIONS.json.
pub fn get_build_details(root: &Path, config_name: &str, dest_name: &str) -> Result<(Vec<(String, String, String)>, Vec<(String, String, Option<String>, String)>)> {
    let versions_file = root.join("VERSIONS.json");
    if !versions_file.exists() {
        return Ok((vec![], vec![]));
    }

    let content = fs::read_to_string(&versions_file)
        .map_err(|e| anyhow!("failed to read {}: {}", versions_file.display(), e))?;
    let data: Value = serde_json::from_str(&content)
        .map_err(|e| anyhow!("failed to parse {}: {}", versions_file.display(), e))?;

    let target_prefix = format!("build:{}/", config_name);
    let mut versions = vec![];
    let mut deployments = vec![];

    // Collect build entries
    if let Some(entries) = data.get("entries").and_then(|e| e.as_array()) {
        for entry in entries {
            if let (Some(target), Some(version), Some(date)) = (
                entry.get("target").and_then(|t| t.as_str()),
                entry.get("version").and_then(|v| v.as_str()),
                entry.get("date").and_then(|d| d.as_str()),
            ) {
                if target.starts_with(&target_prefix) {
                    versions.push((target.to_string(), version.to_string(), date.to_string()));
                }
            }
        }
    }

    // Collect deployments
    let deploy_prefix = format!("deploy:{}/", dest_name);
    if let Some(deploys) = data.get("deployments").and_then(|d| d.as_array()) {
        for deploy in deploys {
            if let (Some(target), Some(date), Some(mode)) = (
                deploy.get("target").and_then(|t| t.as_str()),
                deploy.get("deployed_at").and_then(|d| d.as_str()),
                deploy.get("mode").and_then(|m| m.as_str()),
            ) {
                if target.starts_with(&deploy_prefix) {
                    let version = deploy.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                    deployments.push((target.to_string(), date.to_string(), version, mode.to_string()));
                }
            }
        }
    }

    Ok((versions, deployments))
}

/// Get all deployments for a cluster/container(s) from VERSIONS.json.
pub fn get_deploy_details(root: &Path, cluster: &str, containers: &[String]) -> Result<Vec<(String, String, Option<String>, String)>> {
    let versions_file = root.join("VERSIONS.json");
    if !versions_file.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&versions_file)
        .map_err(|e| anyhow!("failed to read {}: {}", versions_file.display(), e))?;
    let data: Value = serde_json::from_str(&content)
        .map_err(|e| anyhow!("failed to parse {}: {}", versions_file.display(), e))?;

    let mut results = vec![];

    if let Some(deploys) = data.get("deployments").and_then(|d| d.as_array()) {
        for deploy in deploys {
            if let (Some(target), Some(date), Some(mode)) = (
                deploy.get("target").and_then(|t| t.as_str()),
                deploy.get("deployed_at").and_then(|d| d.as_str()),
                deploy.get("mode").and_then(|m| m.as_str()),
            ) {
                let prefix = format!("deploy:{}/", cluster);
                if target.starts_with(&prefix) {
                    // If containers specified, filter to those
                    if !containers.is_empty() {
                        let target_container = &target[prefix.len()..];
                        if !containers.contains(&target_container.to_string()) {
                            continue;
                        }
                    }
                    let version = deploy.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                    results.push((target.to_string(), date.to_string(), version, mode.to_string()));
                }
            }
        }
    }

    Ok(results)
}

/// Find the highest recorded version for a given target in the VERSIONS.json file.
/// Returns `None` if no entry exists for that target.
fn max_version_for_target(versions_file: &Path, target: &str) -> Result<Option<String>> {
    let content = fs::read_to_string(versions_file)
        .map_err(|e| anyhow!("failed to read {}: {}", versions_file.display(), e))?;
    let data: Value = serde_json::from_str(&content)
        .map_err(|e| anyhow!("failed to parse {}: {}", versions_file.display(), e))?;

    let mut max_version: Option<(u64, u64, u64)> = None;
    let mut max_str = String::new();

    if let Some(entries) = data.get("entries").and_then(|e| e.as_array()) {
        for entry in entries {
            if let Some(entry_target) = entry.get("target").and_then(|t| t.as_str()) {
                if entry_target == target {
                    if let Some(entry_version) = entry.get("version").and_then(|v| v.as_str()) {
                        if let Some(parsed) = parse_version(entry_version) {
                            if max_version.is_none() || parsed > max_version.unwrap() {
                                max_version = Some(parsed);
                                max_str = entry_version.to_string();
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(max_version.map(|_| max_str))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("1"), Some((1, 0, 0)));
        assert_eq!(parse_version("1.2.3.4"), None); // too many parts
        assert_eq!(parse_version("1.2.3-rc1"), None); // pre-release
        assert_eq!(parse_version("1.2.3+build"), None); // build metadata
        assert_eq!(parse_version("invalid"), None);
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("2.0.0", "1.0.0"));
        assert!(is_newer("1.2.0", "1.1.0"));
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "2.0.0"));
        assert!(!is_newer("1.0.0-rc1", "1.0.0")); // pre-release never newer
        assert!(!is_newer("invalid", "1.0.0"));
    }

    #[test]
    fn test_pkg_version() {
        let temp = TempDir::new().unwrap();
        let pkg_file = temp.path().join("package.json");
        fs::write(&pkg_file, r#"{"name": "test", "version": "1.2.3"}"#).unwrap();

        let version = pkg_version(temp.path()).unwrap();
        assert_eq!(version, "1.2.3");
    }

    #[test]
    fn test_set_pkg_version() {
        let temp = TempDir::new().unwrap();
        let pkg_file = temp.path().join("package.json");
        fs::write(&pkg_file, r#"{"name": "test", "version": "1.0.0"}"#).unwrap();

        set_pkg_version(temp.path(), "2.0.0").unwrap();
        let content = fs::read_to_string(&pkg_file).unwrap();
        assert!(content.contains("\"version\": \"2.0.0\""));
    }

    #[test]
    fn test_record_and_max_version() {
        let temp = TempDir::new().unwrap();
        record(temp.path(), "build:webapp", "1.0.0", "prod", false).unwrap();
        record(temp.path(), "build:webapp", "1.1.0", "prod", false).unwrap();
        record(temp.path(), "build:desktop", "1.0.0", "prod", false).unwrap();

        let max = max_version_for_target(&temp.path().join("VERSIONS.json"), "build:webapp").unwrap();
        assert_eq!(max, Some("1.1.0".to_string()));

        let max_desktop = max_version_for_target(&temp.path().join("VERSIONS.json"), "build:desktop").unwrap();
        assert_eq!(max_desktop, Some("1.0.0".to_string()));

        let max_none = max_version_for_target(&temp.path().join("VERSIONS.json"), "build:nonexistent").unwrap();
        assert_eq!(max_none, None);
    }

    #[test]
    fn test_check_gate() {
        let temp = TempDir::new().unwrap();
        let pkg_file = temp.path().join("package.json");
        fs::write(&pkg_file, r#"{"name": "test", "version": "1.0.0"}"#).unwrap();

        // Valid: higher version
        assert!(check(temp.path(), "build:test", "1.1.0", temp.path()).is_ok());

        // Invalid: same version
        assert!(check(temp.path(), "build:test", "1.0.0", temp.path()).is_err());

        // Invalid: lower version
        assert!(check(temp.path(), "build:test", "0.9.0", temp.path()).is_err());

        // Invalid: pre-release
        assert!(check(temp.path(), "build:test", "1.1.0-rc1", temp.path()).is_err());
    }

    #[test]
    fn test_check_table_only() {
        let temp = TempDir::new().unwrap();

        // Valid: no prior entry in table, any version is allowed
        assert!(check_table_only(temp.path(), "build:webapp", "0.0.2").is_ok());

        // Record a version to the table
        record(temp.path(), "build:webapp", "0.0.2", "dev", false).unwrap();

        // Invalid: same version as recorded
        assert!(check_table_only(temp.path(), "build:webapp", "0.0.2").is_err());

        // Invalid: lower version than recorded
        assert!(check_table_only(temp.path(), "build:webapp", "0.0.1").is_err());

        // Valid: higher version than recorded
        assert!(check_table_only(temp.path(), "build:webapp", "0.0.3").is_ok());

        // Valid: different target, no prior entry
        assert!(check_table_only(temp.path(), "build:desktop", "0.0.1").is_ok());

        // Invalid: pre-release
        assert!(check_table_only(temp.path(), "build:webapp", "1.0.0-rc1").is_err());
    }
}
