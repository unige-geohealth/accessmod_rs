//! `cargo xtask i18n [--check]`: build the interface catalogue
//! `web/src/i18n/catalog.json` (committed) in en, fr, es and pt from
//!
//! - `web/i18n/am5-keys.json`: interface keys taken from the AccessMod 5
//!   dictionary (`_am5/accessmod/www/dictionary/main.json`), by its ids;
//! - `web/i18n/own.json`: keys AccessMod 5 has no text for, translated here;
//! - the AccessMod 5 data classes (`classes.json`), under `classes`.
//!
//! The AccessMod 5 source (`sh scripts/fetch-am5.sh`) is needed to write or
//! check the catalogue; without it, `--check` is skipped with a message.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::workspace_root;

const LANGUAGES: [&str; 4] = ["en", "fr", "es", "pt"];
const CATALOG: &str = "web/src/i18n/catalog.json";

/// Run the task with its arguments.
pub fn run(args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err(format!("unknown arguments to i18n: {}", args.join(" "))),
    };
    let root = workspace_root();
    let dictionary = root.join("_am5/accessmod/www/dictionary");
    if !dictionary.is_dir() {
        if check {
            eprintln!("i18n: skipped, no AccessMod 5 source (sh scripts/fetch-am5.sh)");
            return Ok(());
        }
        return Err("no AccessMod 5 source: run `sh scripts/fetch-am5.sh`".into());
    }
    let mut text =
        serde_json::to_string_pretty(&catalog(&root, &dictionary)?).map_err(|e| e.to_string())?;
    text.push('\n');
    let path = root.join(CATALOG);
    if check {
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        if committed != text {
            return Err(format!("{CATALOG} is out of date; run `cargo xtask i18n`"));
        }
        eprintln!("{CATALOG} is up to date");
    } else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        eprintln!("wrote {CATALOG}");
    }
    Ok(())
}

fn read(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// AccessMod 5 entries by id: language -> text.
fn entries(path: &Path, id: &str) -> Result<BTreeMap<String, Map<String, Value>>, String> {
    let list = read(path)?;
    Ok(list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let key = e.get(id)?.as_str()?.to_owned();
            Some((key, e.as_object()?.clone()))
        })
        .collect())
}

fn catalog(root: &Path, dictionary: &Path) -> Result<Value, String> {
    let main = entries(&dictionary.join("main.json"), "id")?;
    let classes = entries(&dictionary.join("classes.json"), "class")?;
    let am5_keys = read(&root.join("web/i18n/am5-keys.json"))?;
    let own = read(&root.join("web/i18n/own.json"))?;
    let mut messages: BTreeMap<&str, Map<String, Value>> =
        LANGUAGES.iter().map(|l| (*l, Map::new())).collect();
    for (key, id) in am5_keys.as_object().into_iter().flatten() {
        let id = id.as_str().unwrap_or_default();
        let entry = main
            .get(id)
            .ok_or_else(|| format!("am5-keys.json: {key}: no AccessMod 5 id {id}"))?;
        for language in LANGUAGES {
            // Missing translations fall back to English.
            let text = entry
                .get(language)
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .or_else(|| entry.get("en").and_then(Value::as_str))
                .unwrap_or(key);
            if let Some(m) = messages.get_mut(language) {
                m.insert(key.clone(), json!(text));
            }
        }
    }
    for (key, texts) in own.as_object().into_iter().flatten() {
        for language in LANGUAGES {
            let text = texts
                .get(language)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("own.json: {key} has no {language} text"))?;
            if let Some(m) = messages.get_mut(language) {
                if m.contains_key(key) {
                    return Err(format!("{key} is in both am5-keys.json and own.json"));
                }
                m.insert(key.clone(), json!(text));
            }
        }
    }
    let mut class_names: BTreeMap<&str, Map<String, Value>> =
        LANGUAGES.iter().map(|l| (*l, Map::new())).collect();
    for (class, entry) in &classes {
        for language in LANGUAGES {
            let text = entry
                .get(language)
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .or_else(|| entry.get("en").and_then(Value::as_str))
                .unwrap_or(class);
            if let Some(m) = class_names.get_mut(language) {
                m.insert(class.clone(), json!(text));
            }
        }
    }
    Ok(json!({
        "languages": LANGUAGES,
        "messages": messages,
        "classes": class_names,
    }))
}
