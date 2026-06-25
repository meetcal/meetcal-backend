//! A small JSON-array file store shared by the Slack-managed lists (meet
//! watches and entry targets). Items are objects keyed by a configurable field
//! (`key` for watches, `label` for entries). Operations return
//! `Result<_, String>` where the `Err` string is surfaced back into Slack.
//!
//! Writes are atomic (temp file + rename) so the scraper cron, reading the same
//! file on the same server, never observes a half-written list. Unknown fields
//! are preserved on rewrite, since the file is parsed as generic JSON values.

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct JsonListStore {
    path: PathBuf,
    key_field: &'static str,
}

impl JsonListStore {
    pub fn new(path: PathBuf, key_field: &'static str) -> Self {
        Self { path, key_field }
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
        match serde_json::from_str(&text).map_err(|e| format!("list file is not valid JSON: {e}"))? {
            Value::Array(items) => Ok(items),
            _ => Err("list file must contain a JSON array".to_string()),
        }
    }

    fn save(&self, items: &[Value]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create list directory: {e}"))?;
        }
        let body = serde_json::to_string_pretty(&items)
            .map_err(|e| format!("could not serialize list: {e}"))?;
        let tmp = self.path.with_extension("json.tmp");
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
        let s = JsonListStore::new(dir.0.join("list.json"), "key");
        assert!(s.items().unwrap().is_empty());

        s.add(json!({"key": "a", "meet_name": "A"})).unwrap();
        assert_eq!(s.items().unwrap().len(), 1);
        assert!(s.add(json!({"key": "A"})).is_err()); // dup, case-insensitive
        assert!(s.delete("A").unwrap());
        assert!(!s.delete("missing").unwrap());
    }

    #[test]
    fn rejects_keyless_object() {
        let dir = Tmp::new();
        let s = JsonListStore::new(dir.0.join("list.json"), "key");
        assert!(s.add(json!({"meet_name": "no key"})).is_err());
    }
}
