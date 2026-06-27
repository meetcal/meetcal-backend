//! A small list-file store shared by the Slack-managed lists (meet watches,
//! entry targets, and urlwatch jobs). Items are objects keyed by a configurable
//! field (`key` for watches, `label` for entries, `name` for urlwatch).
//! Operations return `Result<_, String>` where the `Err` string is surfaced
//! back into Slack.
//!
//! Two on-disk formats are supported, chosen per list:
//! * [`ListFormat::Json`] — a single JSON array of objects (`watches.json`,
//!   `entries_targets.json`).
//! * [`ListFormat::Yaml`] — a multi-document YAML stream, one object per `---`
//!   document (`urls.yaml`, consumed by urlwatch).
//!
//! Either way items are handled as generic JSON `Value`s, so unknown fields
//! (e.g. urlwatch `filter:` chains) round-trip untouched. Writes are atomic
//! (temp file + rename) so the scraper cron, reading the same file on the same
//! server, never observes a half-written list.

use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// On-disk encoding of a managed list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListFormat {
    /// A single JSON array of objects.
    Json,
    /// A multi-document YAML stream (`---`-separated), one object per document.
    Yaml,
}

#[derive(Clone)]
pub struct ListStore {
    path: PathBuf,
    key_field: &'static str,
    format: ListFormat,
}

impl ListStore {
    pub fn new(path: PathBuf, key_field: &'static str, format: ListFormat) -> Self {
        Self { path, key_field, format }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn items(&self) -> Result<Vec<Value>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("could not read list file: {e}"))?;
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        match self.format {
            ListFormat::Json => parse_json(&text),
            ListFormat::Yaml => parse_yaml(&text),
        }
    }

    fn save(&self, items: &[Value]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create list directory: {e}"))?;
        }
        let body = match self.format {
            ListFormat::Json => serde_json::to_string_pretty(&items)
                .map_err(|e| format!("could not serialize list: {e}"))?,
            ListFormat::Yaml => serialize_yaml(items)?,
        };
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, format!("{body}\n"))
            .map_err(|e| format!("could not write list file: {e}"))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("could not replace list file: {e}"))?;
        Ok(())
    }

    pub fn key_of(&self, item: &Value) -> String {
        item.get(self.key_field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// Append `object` (must carry the key field) unless its key already exists.
    pub fn add(&self, object: Value) -> Result<(), String> {
        let key = object
            .get(self.key_field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if key.is_empty() {
            return Err(format!("missing `{}`", self.key_field));
        }
        let mut items = self.items()?;
        if items.iter().any(|i| self.key_of(i).eq_ignore_ascii_case(&key)) {
            return Err(format!("an entry with {} `{key}` already exists", self.key_field));
        }
        items.push(object);
        self.save(&items)
    }

    /// Remove the item whose key matches (case-insensitive). Returns whether one
    /// was removed.
    pub fn delete(&self, key: &str) -> Result<bool, String> {
        let mut items = self.items()?;
        let before = items.len();
        items.retain(|i| !self.key_of(i).eq_ignore_ascii_case(key));
        let removed = items.len() != before;
        if removed {
            self.save(&items)?;
        }
        Ok(removed)
    }
}

fn parse_json(text: &str) -> Result<Vec<Value>, String> {
    match serde_json::from_str(text).map_err(|e| format!("list file is not valid JSON: {e}"))? {
        Value::Array(items) => Ok(items),
        _ => Err("list file must contain a JSON array".to_string()),
    }
}

/// Parse a multi-document YAML stream into one `Value` per document, skipping
/// empty/null documents (e.g. a trailing `---`). Each document must be a
/// mapping; YAML maps cleanly onto a JSON object.
fn parse_yaml(text: &str) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(text) {
        let value =
            Value::deserialize(doc).map_err(|e| format!("list file is not valid YAML: {e}"))?;
        match value {
            Value::Null => continue,
            v @ Value::Object(_) => items.push(v),
            _ => return Err("each YAML document must be a mapping".to_string()),
        }
    }
    Ok(items)
}

/// Serialize items back to a `---`-separated multi-document YAML stream.
fn serialize_yaml(items: &[Value]) -> Result<String, String> {
    let docs: Result<Vec<String>, String> = items
        .iter()
        .map(|item| serde_yaml::to_string(item).map_err(|e| format!("could not serialize list: {e}")))
        .collect();
    // `serde_yaml::to_string` already appends a trailing newline per document,
    // so joining on `---\n` yields a valid multi-document stream.
    Ok(docs?.join("---\n").trim_end().to_string())
}

pub fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Validate an optional URL field. `None` passes (the field is absent).
pub fn require_http_url(label: &str, url: Option<&str>) -> Result<(), String> {
    match url {
        Some(u) if !is_http_url(u) => Err(format!("{label} must start with http(s): `{u}`")),
        _ => Ok(()),
    }
}

/// A slug usable as a stable key: letters, numbers, `-`, `_`.
pub fn validate_slug(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "{field} `{value}` may only contain letters, numbers, `-` and `_`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Self {
            let mut p = std::env::temp_dir();
            let n = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            p.push(format!("meetcal-store-test-{n}"));
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn add_list_delete() {
        let dir = Tmp::new();
        let s = ListStore::new(dir.0.join("list.json"), "key", ListFormat::Json);
        assert!(s.items().unwrap().is_empty());

        s.add(json!({"key": "a", "meet_name": "A"})).unwrap();
        assert_eq!(s.items().unwrap().len(), 1);
        assert!(s.add(json!({"key": "A"})).is_err()); // dup, case-insensitive
        assert!(s.delete("A").unwrap());
        assert!(!s.delete("missing").unwrap());
    }

    #[test]
    fn yaml_multidoc_round_trip() {
        let dir = Tmp::new();
        let path = dir.0.join("urls.yaml");
        // Seed a multi-document YAML file with a non-trivial filter chain.
        std::fs::write(
            &path,
            "name: A\nurl: https://a.example/x\nfilter:\n  - css: .foo\n  - html2text\n---\nname: B\nurl: https://b.example/y\nfilter:\n  - html2text\n",
        )
        .unwrap();
        let s = ListStore::new(path.clone(), "name", ListFormat::Yaml);

        let items = s.items().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(s.key_of(&items[0]), "A");

        // Add one, and confirm it persists as a third YAML document.
        s.add(json!({"name": "C", "url": "https://c.example/z", "filter": ["html2text"]}))
            .unwrap();
        assert_eq!(s.items().unwrap().len(), 3);
        assert!(s.add(json!({"name": "a"})).is_err()); // dup, case-insensitive

        // The unknown-shaped filter on A must survive the rewrite untouched.
        let a = s.items().unwrap().into_iter().find(|i| s.key_of(i) == "A").unwrap();
        assert_eq!(a["filter"][0]["css"], ".foo");
        assert_eq!(a["filter"][1], "html2text");

        assert!(s.delete("b").unwrap());
        assert_eq!(s.items().unwrap().len(), 2);
        // File still parses as valid multi-doc YAML after edits.
        assert!(serde_yaml::Deserializer::from_str(&std::fs::read_to_string(&path).unwrap())
            .count() >= 2);
    }

    #[test]
    fn rejects_keyless_object() {
        let dir = Tmp::new();
        let s = ListStore::new(dir.0.join("list.json"), "key", ListFormat::Json);
        assert!(s.add(json!({"meet_name": "no key"})).is_err());
    }
}
