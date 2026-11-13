//! `cargo xtask i18n [--check]`: build the interface catalogue
//! `design_system/design-system/src/i18n/catalog.json` (committed): one
//! object per language (en, fr, es, pt), key to text, from
//!
//! - the AccessMod 5 dictionary (`_am5/accessmod/www/dictionary/main.json`),
//!   all entries, under their AccessMod 5 ids unchanged (the ids are the
//!   `data-amt_id` of `design_system/references/screens/*/dom.html`);
//! - the AccessMod 5 data classes (`classes.json`), as `class.<class>`;
//! - `design_system/design-system/src/i18n/own.json`: keys AccessMod 5 has
//!   no text for, with dotted names (`table.all`), in all four languages.
//!
//! It also writes `crates/accessmod-io/src/am5_classes.json` (committed):
//! the class labels of `classes.json` in every language, as they are,
//! which name the files of an AccessMod 5 data archive (`accessmod_io::am5`).
//!
//! A missing AccessMod 5 translation falls back to English. An own key may
//! not shadow an AccessMod 5 id.
//!
//! The AccessMod 5 source (`sh scripts/fetch-am5.sh`) is needed to write or
//! check the catalogue; without it, `--check` is skipped with a message.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::workspace_root;

const LANGUAGES: [&str; 4] = ["en", "fr", "es", "pt"];
const CATALOG: &str = "design_system/design-system/src/i18n/catalog.json";
const OWN: &str = "design_system/design-system/src/i18n/own.json";
const CLASSES: &str = "crates/accessmod-io/src/am5_classes.json";

type Catalog = BTreeMap<&'static str, BTreeMap<String, String>>;

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
    let classes = read(&dictionary.join("classes.json"))?;
    let catalog = catalog(
        &read(&dictionary.join("main.json"))?,
        &classes,
        &read(&root.join(OWN))?,
    )?;
    let outputs = [
        (CATALOG, pretty(serde_json::to_string_pretty(&catalog))?),
        (
            CLASSES,
            pretty(serde_json::to_string_pretty(&class_labels(&classes)))?,
        ),
    ];
    for (file, text) in outputs {
        let path = root.join(file);
        if check {
            let committed = std::fs::read_to_string(&path).unwrap_or_default();
            if committed != text {
                return Err(format!("{file} is out of date; run `cargo xtask i18n`"));
            }
            eprintln!("{file} is up to date");
        } else {
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
            eprintln!("wrote {file}");
        }
    }
    Ok(())
}

fn pretty(text: serde_json::Result<String>) -> Result<String, String> {
    let mut text = text.map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(text)
}

/// Class, type and labels of every AccessMod 5 class, as `classes.json`
/// has them (no fallback: AccessMod 5 names files with the label as is).
fn class_labels(classes: &Value) -> Vec<BTreeMap<&'static str, String>> {
    let mut out: Vec<BTreeMap<&'static str, String>> = classes
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let field = |k: &str| {
                entry
                    .get(k)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            let class = field("class");
            if class.is_empty() {
                return None;
            }
            let mut row = BTreeMap::from([("class", class), ("type", field("type"))]);
            for language in LANGUAGES {
                row.insert(language, field(language));
            }
            Some(row)
        })
        .collect();
    out.sort_by(|a, b| a["class"].cmp(&b["class"]));
    out
}

fn read(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Text of `entry` in `language`, else English, else `fallback`.
fn text<'a>(entry: &'a Value, language: &str, fallback: &'a str) -> &'a str {
    let get = |l: &str| {
        entry
            .get(l)
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
    };
    get(language).or_else(|| get("en")).unwrap_or(fallback)
}

/// Add the AccessMod 5 entries of `list`, keyed by their `id` field.
fn add_am5(catalog: &mut Catalog, list: &Value, id: &str, prefix: &str) {
    for entry in list.as_array().into_iter().flatten() {
        let Some(key) = entry.get(id).and_then(Value::as_str) else {
            continue;
        };
        let key = format!("{prefix}{key}");
        for language in LANGUAGES {
            let value = text(entry, language, &key).to_owned();
            catalog
                .entry(language)
                .or_default()
                .insert(key.clone(), value);
        }
    }
}

/// Catalogue from the AccessMod 5 `main.json` and `classes.json` and `own.json`.
fn catalog(main: &Value, classes: &Value, own: &Value) -> Result<Catalog, String> {
    let mut catalog: Catalog = LANGUAGES.iter().map(|l| (*l, BTreeMap::new())).collect();
    add_am5(&mut catalog, main, "id", "");
    add_am5(&mut catalog, classes, "class", "class.");
    for (key, texts) in own.as_object().into_iter().flatten() {
        if catalog["en"].contains_key(key) {
            return Err(format!("{OWN}: {key} shadows an AccessMod 5 id"));
        }
        for language in LANGUAGES {
            let value = texts
                .get(language)
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| format!("{OWN}: {key} has no {language} text"))?;
            catalog
                .entry(language)
                .or_default()
                .insert(key.clone(), value.to_owned());
        }
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn am5_ids_classes_and_own_keys() {
        let main = json!([{ "id": "main_ui_tab_data", "en": "Data", "fr": "Données", "es": "", "pt": "Dados" }]);
        let classes =
            json!([{ "class": "rDem", "en": "dem", "fr": "dem", "es": "mde", "pt": "mde" }]);
        let own = json!({ "table.all": { "en": "All", "fr": "Tout", "es": "Todo", "pt": "Tudo" } });
        let c = catalog(&main, &classes, &own).unwrap();
        assert_eq!(c["fr"]["main_ui_tab_data"], "Données");
        assert_eq!(
            c["es"]["main_ui_tab_data"], "Data",
            "empty falls back to English"
        );
        assert_eq!(c["es"]["class.rDem"], "mde");
        assert_eq!(c["pt"]["table.all"], "Tudo");
    }

    #[test]
    fn own_key_cannot_shadow_or_miss_a_language() {
        let (main, classes) = (json!([{ "id": "a", "en": "A" }]), json!([]));
        let shadow = json!({ "a": { "en": "x", "fr": "x", "es": "x", "pt": "x" } });
        assert!(
            catalog(&main, &classes, &shadow)
                .unwrap_err()
                .contains("shadows")
        );
        let partial = json!({ "b.c": { "en": "x", "fr": "x", "es": "x" } });
        assert!(
            catalog(&main, &classes, &partial)
                .unwrap_err()
                .contains("no pt text")
        );
    }
}
