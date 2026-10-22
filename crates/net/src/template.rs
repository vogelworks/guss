//! Resolves `!include` and `!template` tags in a `vn-network-launch` staging
//! template and emits the fully-assembled CloudFormation YAML.
//!
//! `!include` is ported from `vn-network-launch/build-kit/{template-builder,anyyml}.py`
//! (recovered from that repo's git history — `build-kit/` is gitignored there, so only
//! stale `.pyc` bytecode remains on disk). The Python version is a custom PyYAML
//! `Loader`/`Dumper`: any scalar tagged `!include <relpath>` is replaced by the fully
//! parsed contents of that file (path resolved relative to the *including* file's own
//! directory, recursively), while every other tag (CloudFormation's `!Ref`, `!Sub`,
//! `!GetAtt`, ...) round-trips through unchanged so the dumped template stays valid
//! CloudFormation. `saphyr` wraps any node under an unrecognized tag in `Tagged`,
//! which gives the same round-trip behavior directly — no `Tagged`-proxy plumbing
//! needed the way the Python version required `wrapt`.
//!
//! `!template` is a second, sibling mechanism for the same "shareable staging file"
//! goal: several `*LaunchTemplate.yml` resources embed a `UserData` bash script that's
//! ~95% identical boilerplate across clusters, differing only in a couple of values
//! (cluster name, the docker-compose body). `!template` factors that boilerplate into
//! one shared text file with `{{VAR}}` placeholders, resolved at build time:
//!
//! ```yaml
//! Fn::Base64: !template
//!   file: ../../templates/docker-compose-userdata.sh
//!   sub: true
//!   CLUSTER: mart
//!   COMPOSE: |
//!     mongomart:
//!       image: cmv343/guss-mongomart:dev
//! ```
//!
//! `file` (required) is resolved relative to the including file's directory, same
//! rule as `!include`. Every other key is a substitution variable: its resolved value
//! (recursively resolving any nested `!include`/`!template` first) must be a plain
//! string, and every `{{KEY}}` in the template file is replaced with it — an
//! unreplaced `{{...}}` left over is an error (most likely a typo'd variable name).
//! `sub` (optional, default `false`) wraps the substituted result in a `!Sub` tag,
//! since CloudFormation's own `${environment}`-style interpolation happens at deploy
//! time, after this resolves — `{{VAR}}` was chosen specifically so it can never be
//! confused with `!Sub`'s `${...}` syntax already present in these scripts.

use guss_common::error::{Result, anyhow};
use saphyr::{LoadableYamlNode, MappingOwned, Tag, Yaml, YamlEmitter, YamlOwned};
use std::fs;
use std::path::Path;

fn is_include_tag(tag: &Tag) -> bool {
    tag.handle == "!" && tag.suffix == "include"
}

fn is_template_tag(tag: &Tag) -> bool {
    tag.handle == "!" && tag.suffix == "template"
}

fn sub_tag() -> Tag {
    Tag {
        handle: "!".to_string(),
        suffix: "Sub".to_string(),
    }
}

/// Resolve a `!template` mapping (already include-resolved) into the substituted
/// string, optionally wrapped in `!Sub`.
fn resolve_template(node: YamlOwned, dir: &Path) -> Result<YamlOwned> {
    let map = match node {
        YamlOwned::Mapping(map) => map,
        _ => {
            return Err(anyhow!(
                "!template in {} must be a mapping with a `file` key and substitution variables",
                dir.display()
            ));
        }
    };

    let mut file: Option<String> = None;
    let mut wrap_sub = false;
    let mut vars: Vec<(String, String)> = Vec::new();
    for (k, v) in map {
        let key = k
            .as_str()
            .ok_or_else(|| anyhow!("!template keys in {} must be strings", dir.display()))?
            .to_string();
        match key.as_str() {
            "file" => {
                file = Some(
                    v.as_str()
                        .ok_or_else(|| anyhow!("!template `file` in {} must be a string", dir.display()))?
                        .to_string(),
                );
            }
            "sub" => {
                wrap_sub = matches!(v, YamlOwned::Value(saphyr::ScalarOwned::Boolean(true)));
            }
            _ => {
                let value = v
                    .as_str()
                    .ok_or_else(|| anyhow!("!template variable `{key}` in {} must be a plain string", dir.display()))?
                    .to_string();
                vars.push((key, value));
            }
        }
    }

    let file = file.ok_or_else(|| anyhow!("!template mapping in {} is missing `file`", dir.display()))?;
    let template_path = dir.join(file);
    let mut text = fs::read_to_string(&template_path).map_err(|e| anyhow!("reading template {}: {e}", template_path.display()))?;
    for (name, value) in &vars {
        text = text.replace(&format!("{{{{{name}}}}}"), value);
    }
    if let Some(start) = text.find("{{") {
        let end = (start + 40).min(text.len());
        return Err(anyhow!("unresolved placeholder in {}: {}...", template_path.display(), &text[start..end]));
    }

    let value = YamlOwned::Value(saphyr::ScalarOwned::String(text));
    Ok(if wrap_sub {
        YamlOwned::Tagged(sub_tag(), Box::new(value))
    } else {
        value
    })
}

/// Parse `path` and recursively resolve every `!include` tag it contains.
fn resolve(path: &Path) -> Result<YamlOwned> {
    let contents = fs::read_to_string(path).map_err(|e| anyhow!("reading {}: {e}", path.display()))?;
    let docs = YamlOwned::load_from_str(&contents).map_err(|e| anyhow!("parsing {}: {e}", path.display()))?;
    let root = docs.into_iter().next().unwrap_or(YamlOwned::BadValue);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    resolve_includes(root, dir)
}

/// Walk a parsed tree, replacing `!include <relpath>` scalars with the resolved
/// contents of `<relpath>` (resolved relative to `dir`, the including file's own
/// directory — mirrors `Loader.include`'s `self._root`).
fn resolve_includes(node: YamlOwned, dir: &Path) -> Result<YamlOwned> {
    match node {
        YamlOwned::Tagged(tag, inner) if is_include_tag(&tag) => {
            let rel = inner
                .as_str()
                .ok_or_else(|| anyhow!("!include in {} is not a plain string", dir.display()))?
                .to_string();
            resolve(&dir.join(rel))
        }
        YamlOwned::Tagged(tag, inner) if is_template_tag(&tag) => {
            let resolved_inner = resolve_includes(*inner, dir)?;
            resolve_template(resolved_inner, dir)
        }
        YamlOwned::Tagged(tag, inner) => Ok(YamlOwned::Tagged(tag, Box::new(resolve_includes(*inner, dir)?))),
        YamlOwned::Sequence(items) => {
            let resolved = items.into_iter().map(|item| resolve_includes(item, dir)).collect::<Result<Vec<_>>>()?;
            Ok(YamlOwned::Sequence(resolved))
        }
        YamlOwned::Mapping(map) => {
            let mut resolved = MappingOwned::new();
            for (k, v) in map {
                resolved.insert(resolve_includes(k, dir)?, resolve_includes(v, dir)?);
            }
            Ok(YamlOwned::Mapping(resolved))
        }
        other => Ok(other),
    }
}

/// Sort mapping keys alphabetically, matching PyYAML `SafeDumper`'s default output
/// order (the Python `Dumper` never overrode key ordering) — keeps the Rust output
/// diffable against the existing `staging/builds/new-build.yml`.
fn sort_mapping_keys(node: YamlOwned) -> YamlOwned {
    match node {
        YamlOwned::Mapping(map) => {
            let mut entries: Vec<_> = map.into_iter().map(|(k, v)| (sort_mapping_keys(k), sort_mapping_keys(v))).collect();
            entries.sort_by(|a, b| a.0.as_str().unwrap_or_default().cmp(b.0.as_str().unwrap_or_default()));
            let mut sorted = MappingOwned::new();
            for (k, v) in entries {
                sorted.insert(k, v);
            }
            YamlOwned::Mapping(sorted)
        }
        YamlOwned::Sequence(items) => YamlOwned::Sequence(items.into_iter().map(sort_mapping_keys).collect()),
        YamlOwned::Tagged(tag, inner) => YamlOwned::Tagged(tag, Box::new(sort_mapping_keys(*inner))),
        other => other,
    }
}

fn emit(node: &YamlOwned) -> Result<String> {
    let sorted = sort_mapping_keys(node.clone());
    let borrowed: Yaml = (&sorted).into();
    let mut out = String::new();
    YamlEmitter::new(&mut out).dump(&borrowed)?;
    Ok(out)
}

/// Resolve `master`'s includes and write the assembled template to `out`. Under
/// `dry_run`, prints the result instead of writing it. Mirrors
/// `template-builder.py`'s load → dump.
pub fn build(master: &Path, out: &Path, dry_run: bool) -> Result<()> {
    let resolved = resolve(master)?;
    let text = emit(&resolved)?;
    if dry_run {
        println!("{text}");
        return Ok(());
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, contents: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn resolves_include_and_preserves_other_tags() {
        let dir = std::env::temp_dir().join(format!("guss-net-template-test-{:?}", std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write(&dir, "master.yml", "Resources:\n  thing: !include ./resources/thing.yml\n");
        write(&dir, "resources/thing.yml", "Type: 'AWS::EC2::SecurityGroup'\nGroupId: !Ref someGroup\n");

        let resolved = resolve(&dir.join("master.yml")).unwrap();
        let text = emit(&resolved).unwrap();

        assert!(text.contains("AWS::EC2::SecurityGroup"));
        assert!(text.contains("!Ref someGroup"));
        assert!(!text.contains("!include"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nested_include_resolves_relative_to_included_files_own_directory() {
        let dir = std::env::temp_dir().join(format!("guss-net-template-test-nested-{:?}", std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write(&dir, "master.yml", "Resources:\n  outer: !include ./a/outer.yml\n");
        write(&dir, "a/outer.yml", "Inner: !include ./inner.yml\n");
        write(&dir, "a/inner.yml", "Value: 42\n");

        let resolved = resolve(&dir.join("master.yml")).unwrap();
        let text = emit(&resolved).unwrap();

        assert!(text.contains("Value: 42"));
        assert!(!text.contains("!include"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn template_substitutes_vars_and_wraps_sub_when_requested() {
        let dir = std::env::temp_dir().join(format!("guss-net-template-test-tmpl-{:?}", std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write(&dir, "templates/userdata.sh", "#!/bin/bash -xe\nsername={{CLUSTER}}\n{{COMPOSE}}\n");

        // Two "clusters" sharing the same template — only CLUSTER/COMPOSE differ,
        // mirroring the real mart/reports/etc. duplication.
        for (cluster, image) in [("mart", "guss-mongomart"), ("reports", "guss-reports")] {
            write(
                &dir,
                &format!("resources/{cluster}/launch.yml"),
                &format!(
                    "UserData:\n  Fn::Base64: !template\n    file: ../../templates/userdata.sh\n    sub: true\n    CLUSTER: {cluster}\n    COMPOSE: |\n      image: cmv343/{image}:dev\n"
                ),
            );
        }
        write(
            &dir,
            "master.yml",
            "Resources:\n  mart: !include ./resources/mart/launch.yml\n  reports: !include ./resources/reports/launch.yml\n",
        );

        let resolved = resolve(&dir.join("master.yml")).unwrap();
        let text = emit(&resolved).unwrap();

        assert!(text.contains("sername=mart"));
        assert!(text.contains("sername=reports"));
        assert!(text.contains("cmv343/guss-mongomart:dev"));
        assert!(text.contains("cmv343/guss-reports:dev"));
        assert!(text.contains("!Sub"));
        assert!(!text.contains("{{"));
        assert!(!text.contains("!template"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn template_errors_on_unresolved_placeholder() {
        let dir = std::env::temp_dir().join(format!("guss-net-template-test-tmpl-err-{:?}", std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write(&dir, "templates/userdata.sh", "sername={{CLUSTER}}\n");
        write(&dir, "master.yml", "UserData: !template\n  file: ./templates/userdata.sh\n");

        let err = resolve(&dir.join("master.yml")).unwrap_err();
        assert!(err.to_string().contains("unresolved placeholder"));

        fs::remove_dir_all(&dir).unwrap();
    }
}
