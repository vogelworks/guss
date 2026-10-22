//! `docs` task: generate and search documentation from configured projects.
//!
//! - `docs -b [project]` — generate docs for named project, or all if omitted.
//! - `docs <term>` — search in global root/docs.
//! - `docs <term> --in <config>` — search in specific doc config.

mod builder;

use chrono::Local;
use clap::Args;
use guss_common::config;
use guss_common::error::{Result, anyhow};
use guss_common::{paths, ui, viewer};
use std::fs;

#[derive(Args)]
pub struct DocsArgs {
    /// Build docs (use without project to build all).
    #[arg(short, long)]
    build: bool,
    /// Search term.
    term: Option<String>,
    /// Doc config name (optional, defaults to global root/docs).
    #[arg(long)]
    r#in: Option<String>,
}

pub fn run(args: DocsArgs) -> Result<()> {
    if args.build {
        let settings = config::Settings::load()?;
        if let Some(name) = args.r#in {
            let cfg = settings.get_doc_config(&name)?;
            let date = Local::now().format("%Y-%m-%d").to_string();
            let out = builder::build(&cfg, &date)?;
            ui::good(&format!("wrote {}", out.display()));
        } else {
            let configs = settings.doc_configs()?;
            if configs.is_empty() {
                return Err(anyhow!("no doc_configs configured"));
            }
            let date = Local::now().format("%Y-%m-%d").to_string();
            for cfg in &configs {
                let out = builder::build(&cfg, &date)?;
                ui::good(&format!("wrote {}", out.display()));
            }
        }
        Ok(())
    } else if let Some(term) = args.term {
        if let Some(config) = args.r#in {
            search_config(&term, config)
        } else {
            search_default(&term)
        }
    } else {
        Err(anyhow!("usage: guss docs [-b [project]] | guss docs <term> [--in <config>]"))
    }
}

/// Search docs in a specific config's root folder.
fn search_config(term: &str, config_name: String) -> Result<()> {
    let settings = config::Settings::load()?;
    let cfg = settings.get_doc_config(&config_name)?;

    let root = cfg
        .root
        .as_ref()
        .ok_or_else(|| anyhow!("doc config '{}' must have root folder defined", config_name))?;
    let search_root = if root.is_absolute() {
        root.clone()
    } else {
        let base = settings.root.clone().unwrap_or(paths::project_guss()?);
        base.join(root)
    };

    search_folder(term, &search_root)
}

/// Search docs in the global root/docs folder (default).
fn search_default(term: &str) -> Result<()> {
    let settings = config::Settings::load()?;
    let root = settings.root.clone().unwrap_or(paths::project_guss()?);
    let search_root = root.join("docs");

    if !search_root.is_dir() {
        return Err(anyhow!("docs folder not found at {}", search_root.display()));
    }

    search_folder(term, &search_root)
}

struct SearchResult {
    file_path: String,
    lines: Vec<String>,
}

/// Search docs by any line content within a section, across all markdown files in folder.
fn search_folder(initial_term: &str, search_root: &std::path::Path) -> Result<()> {
    if !search_root.is_dir() {
        return Err(anyhow!("doc root folder not found: {}", search_root.display()));
    }

    println!("Searching in: {}", search_root.display());
    println!("Search term: '{}'\n", initial_term);

    let mut paths = Vec::new();
    collect_md(search_root, &mut paths)?;
    paths.sort();

    if paths.is_empty() {
        return Err(anyhow!("No markdown files found in {}", search_root.display()));
    }

    println!("Found {} markdown files\n", paths.len());

    let all_files: Vec<_> = paths
        .into_iter()
        .filter_map(|path| {
            let content = fs::read_to_string(&path).ok()?;
            let rel = path.strip_prefix(search_root).unwrap_or(&path).to_path_buf();
            Some((rel, content))
        })
        .collect();

    print_results(initial_term, &perform_search(initial_term, &all_files));

    viewer::refine(|term| {
        println!();
        print_results(term, &perform_search(term, &all_files));
    })
}

/// Recursively collect every `.md` file under `dir`.
fn collect_md(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_md(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
    Ok(())
}

fn print_results(term: &str, results: &[SearchResult]) {
    if results.is_empty() {
        println!("no matches found for '{}'", term);
        return;
    }
    for (idx, result) in results.iter().enumerate() {
        if idx > 0 {
            println!();
        }
        println!("════════════════════════════════════════");
        println!("  {}", result.file_path);
        println!("════════════════════════════════════════");
        for line in &result.lines {
            println!("{}", line);
        }
        println!("════════════════════════════════════════");
    }
}

/// Markdown header level (1-3) of a line, or 0 if it isn't a header.
fn header_level(line: &str) -> usize {
    if line.starts_with("### ") {
        3
    } else if line.starts_with("## ") {
        2
    } else if line.starts_with("# ") {
        1
    } else {
        0
    }
}

fn perform_search(needle: &str, files: &[(std::path::PathBuf, String)]) -> Vec<SearchResult> {
    let needle_lower = needle.to_lowercase();
    let mut results = Vec::new();

    for (file_path, content) in files {
        let lines: Vec<&str> = content.lines().collect();

        // Check if document has any headers
        let has_headers = lines.iter().any(|l| header_level(l) > 0);

        if !has_headers {
            // Document without headers: treat entire document as one block
            let document_matches = lines.iter().any(|l| l.to_lowercase().contains(&needle_lower));

            if document_matches {
                results.push(SearchResult {
                    file_path: file_path.display().to_string(),
                    lines: lines.iter().map(|s| s.to_string()).collect(),
                });
            }
        } else {
            // Document with headers: find matches at the deepest level only
            let mut deepest_level: Option<usize> = None;
            let mut matches = Vec::new();

            for (i, line) in lines.iter().enumerate() {
                let level = header_level(line);

                if level > 0 {
                    let section_end = lines[i + 1..]
                        .iter()
                        .position(|l| (1..=level).contains(&header_level(l)))
                        .map(|p| i + 1 + p)
                        .unwrap_or(lines.len());

                    let section_matches = lines[i..section_end].iter().any(|l| l.to_lowercase().contains(&needle_lower));

                    if section_matches {
                        match deepest_level {
                            None => {
                                deepest_level = Some(level);
                                matches.push((i, section_end));
                            }
                            Some(current) => {
                                if level > current {
                                    // Found deeper level, clear previous matches
                                    deepest_level = Some(level);
                                    matches.clear();
                                    matches.push((i, section_end));
                                } else if level == current {
                                    // Same level, add it
                                    matches.push((i, section_end));
                                }
                                // Ignore shallower matches
                            }
                        }
                    }
                }
            }

            // Add final matches to results
            for (start, end) in matches {
                results.push(SearchResult {
                    file_path: file_path.display().to_string(),
                    lines: lines[start..end].iter().map(|s| s.to_string()).collect(),
                });
            }
        }
    }

    results
}
