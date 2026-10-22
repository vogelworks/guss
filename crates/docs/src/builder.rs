//! Config-driven documentation generation for API projects.

use guss_common::config::{DocBuild, DocConfig, Settings};
use guss_common::error::{Result, anyhow};
use guss_common::paths;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn build(cfg: &DocConfig, date: &str) -> Result<PathBuf> {
    let settings = Settings::load()?;
    let config_root = settings.root.clone().unwrap_or(paths::project_guss()?);

    let builds = cfg.builds.as_ref().ok_or_else(|| anyhow!("no builds configured"))?;

    // Resolve output folder (where docs go)
    let output_root = cfg
        .root
        .as_ref()
        .ok_or_else(|| anyhow!("doc config '{}' must have root folder defined", cfg.name))?;
    let output_base = if output_root.is_absolute() {
        output_root.clone()
    } else {
        config_root.join(output_root)
    };

    for build in builds {
        // Resolve build root (where source files are)
        let build_root = build
            .root
            .as_ref()
            .ok_or_else(|| anyhow!("build type '{}' must have root folder defined", build.r#type))?;

        let source_root = if build_root.is_absolute() {
            build_root.clone()
        } else {
            config_root.join(build_root)
        };

        match build.r#type.as_str() {
            "vapi" => {
                let servers = resolve_servers(&source_root, &build.servers)?;
                if servers.is_empty() {
                    eprintln!("warning: no servers found matching {} in {}", build.servers, source_root.display());
                    continue;
                }

                let mut combined = format!("# {}\nGenerated: {date}\n\n---\n\n", cfg.name);
                for server in &servers {
                    match build_docs_content(server, build) {
                        Ok((name, content)) => {
                            combined.push_str(&format!("# {name}\n\n"));
                            combined.push_str(&content);
                        }
                        Err(e) => eprintln!("skip {}: {e}", server.display()),
                    }
                }

                // Determine output path for this build
                let out_filename = build
                    .output
                    .as_ref()
                    .map(|p| p.clone())
                    .unwrap_or_else(|| format!("{}.md", build.r#type).into());
                let out_path = output_base.join(&out_filename);

                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&out_path, combined)?;
            }
            _ => eprintln!("unknown build type: {}", build.r#type),
        }
    }

    Ok(output_base)
}

fn resolve_servers(root: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
    let prefix = pattern.trim_end_matches('*');
    let suffix = if pattern.ends_with('*') { "" } else { pattern };

    let mut dirs: Vec<PathBuf> = fs::read_dir(root)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            if !p.is_dir() {
                return false;
            }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with(prefix) && name.ends_with(suffix)
        })
        .collect();
    dirs.sort();
    Ok(dirs)
}

fn build_docs_content(server: &Path, build: &DocBuild) -> Result<(String, String)> {
    let scan_dir = server.join(&build.scan);
    if !scan_dir.is_dir() {
        return Err(anyhow!("no {} directory", build.scan));
    }

    let pairs = collect_controller_pairs(&scan_dir, build);
    if pairs.is_empty() {
        return Err(anyhow!("no files matching {} found", build.file));
    }

    let server_name = server.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string();

    let mut out = String::new();
    for (ctrl_path, cfg_path) in &pairs {
        let ctrl_name = ctrl_path.file_stem().and_then(|n| n.to_str()).unwrap_or("Unknown");

        let routes = cfg_path.as_ref().map(|p| parse_routes(p, build)).unwrap_or_default();
        if routes.is_empty() {
            continue;
        }

        let docs = extract_docs(ctrl_path, &routes, build);
        out.push_str(&format!("## {ctrl_name}\n\n"));
        for route in &routes {
            let body = docs.get(route).map(|s| s.as_str()).unwrap_or(&build.docs.fallback);
            out.push_str(&format!("### {route}\n\n{body}\n\n---\n\n"));
        }
    }
    Ok((server_name, out))
}

fn collect_controller_pairs(dir: &Path, build: &DocBuild) -> Vec<(PathBuf, Option<PathBuf>)> {
    let mut pairs = Vec::new();
    walk_controllers(dir, &mut pairs, build);
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

fn walk_controllers(dir: &Path, pairs: &mut Vec<(PathBuf, Option<PathBuf>)>, build: &DocBuild) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let mut controllers = Vec::new();
    let mut subdirs = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            let skip = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| build.skip_dirs.contains(&n.to_string()))
                .unwrap_or(false);
            if !skip {
                subdirs.push(path);
            }
        } else {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches_file_pattern(name, &build.file) {
                controllers.push(path);
            }
        }
    }

    for ctrl in controllers {
        let cfg_path = sibling_config(dir, &ctrl, build);
        pairs.push((ctrl, cfg_path));
    }
    for sub in subdirs {
        walk_controllers(&sub, pairs, build);
    }
}

fn matches_file_pattern(name: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }
    name == pattern
}

fn sibling_config(dir: &Path, ctrl: &Path, build: &DocBuild) -> Option<PathBuf> {
    let stem = ctrl.file_stem()?.to_str()?;
    let prefix = stem.trim_end_matches("Controller").to_lowercase();
    let pattern = &build.config.file;
    let config_file = pattern.replace("{stem}", &prefix);
    let candidate = dir.join(&config_file);
    candidate.exists().then_some(candidate)
}

fn parse_routes(path: &Path, build: &DocBuild) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return vec![];
    };

    let mut routes = Vec::new();
    let mut in_routes = false;
    for line in content.lines() {
        let t = line.trim();
        if !in_routes {
            if t.contains(&build.config.routes_start) && t.contains('[') {
                in_routes = true;
            }
            continue;
        }
        if let Some(name) = extract_quoted_name(t, &build.config.route_name) {
            routes.push(name);
        }
        if t == "]" || t == "]," {
            break;
        }
    }
    routes
}

fn extract_quoted_name(line: &str, marker: &str) -> Option<String> {
    let idx = line.find(marker)?;
    let rest = line[idx + marker.len()..].trim_start();
    let q = match rest.chars().next()? {
        c @ ('\'' | '"') => c,
        _ => return None,
    };
    let inner = &rest[1..];
    let end = inner.find(q)?;
    Some(inner[..end].to_string())
}

fn extract_docs(path: &Path, routes: &[String], build: &DocBuild) -> HashMap<String, String> {
    let Ok(content) = fs::read_to_string(path) else {
        return HashMap::new();
    };

    let mut map = HashMap::new();
    let mut doc_buf: Vec<String> = Vec::new();
    let mut in_doc = false;
    let mut doc_ready = false;

    for line in content.lines() {
        let t = line.trim();
        if t.starts_with(&build.docs.block_start) {
            doc_buf.clear();
            in_doc = true;
            doc_ready = false;
            continue;
        }
        if in_doc {
            if t.starts_with(&build.docs.block_end) {
                in_doc = false;
                doc_ready = true;
            } else {
                let stripped = t.strip_prefix("* ").or_else(|| t.strip_prefix('*')).unwrap_or(t);
                doc_buf.push(stripped.to_string());
            }
            continue;
        }
        if doc_ready {
            if t.is_empty() || t.starts_with("//") {
                continue;
            }
            let matched = routes.iter().find(|r| is_method_line(t, r, build));
            if let Some(route) = matched {
                let body = doc_buf.join("\n").trim().to_string();
                map.insert(route.clone(), body);
            }
            doc_ready = false;
            doc_buf.clear();
        }
    }
    map
}

fn is_method_line(line: &str, route: &str, build: &DocBuild) -> bool {
    let pattern = build.docs.method_pattern.replace("{route}", route);
    let trimmed = line.trim();
    trimmed.starts_with(&pattern) || trimmed == &pattern
}
