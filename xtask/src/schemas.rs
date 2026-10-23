//! `cargo xtask schemas [--check]`: write the JSON Schemas the web front end
//! turns into TypeScript types, under `web/schemas/` (committed). With
//! `--check`, fail when a committed file differs or is missing.
//!
//! - `jobs/<analysis>.json`: job document of each analysis;
//! - `data-classes.json`: every data class with its kind, label and whether
//!   it is importable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use accessmod_io::analysis::job_schemas;
use accessmod_io::class::DataClass;
use serde_json::{Value, json};

use crate::workspace_root;

const DIR: &str = "web/schemas";

/// Run the task with its arguments.
pub fn run(args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err(format!("unknown arguments to schemas: {}", args.join(" "))),
    };
    let root = workspace_root().join(DIR);
    let files = files();
    if check {
        let mut stale = Vec::new();
        for (path, text) in &files {
            let full = root.join(path);
            if std::fs::read_to_string(&full).ok().as_deref() != Some(text.as_str()) {
                stale.push(path.display().to_string());
            }
        }
        let extra = existing(&root)?
            .into_iter()
            .filter(|p| !files.contains_key(p))
            .map(|p| format!("{} (not generated)", p.display()));
        stale.extend(extra);
        if !stale.is_empty() {
            return Err(format!(
                "{DIR} is out of date: {}; run `cargo xtask schemas`",
                stale.join(", ")
            ));
        }
        eprintln!("{DIR}: {} files up to date", files.len());
    } else {
        for path in existing(&root)? {
            if !files.contains_key(&path) {
                std::fs::remove_file(root.join(&path))
                    .map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
            }
        }
        for (path, text) in &files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            std::fs::write(&full, text)
                .map_err(|e| format!("cannot write {}: {e}", full.display()))?;
        }
        eprintln!("wrote {} files under {DIR}", files.len());
    }
    Ok(())
}

/// Generated files: path relative to [`DIR`] -> content.
fn files() -> BTreeMap<PathBuf, String> {
    let mut files = BTreeMap::new();
    for (name, schema) in job_schemas() {
        files.insert(PathBuf::from(format!("jobs/{name}.json")), pretty(&schema));
    }
    let classes: Vec<Value> = DataClass::ALL
        .iter()
        .map(|c| {
            json!({
                "class": c.code(),
                "kind": c.kind(),
                "label": c.label(),
                "importable": c.importable(),
            })
        })
        .collect();
    files.insert(PathBuf::from("data-classes.json"), pretty(&json!(classes)));
    files
}

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_default();
    text.push('\n');
    text
}

/// JSON files under `root`, relative to it.
fn existing(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry
                .map_err(|e| format!("cannot list {}: {e}", dir.display()))?
                .path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "json")
                && let Ok(relative) = path.strip_prefix(root)
            {
                out.push(relative.to_path_buf());
            }
        }
    }
    Ok(out)
}
