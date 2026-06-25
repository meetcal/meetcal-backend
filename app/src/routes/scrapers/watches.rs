//! Read/modify the meet-automation `watches.json` that the Python pipeline
//! consumes. Operations return `Result<_, String>` where the `Err` string is a
//! user-facing message surfaced back into Slack (so the caller replies 200 with
//! the text rather than an opaque HTTP error).

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Default schema values mirrored from the Python `MeetWatch` dataclass, so a
/// watch added here is complete for the pipeline.
const DEFAULT_SOURCE_FORMAT: &str = "auto";
const DEFAULT_START_MEMBER_ID: i64 = 3100;
const DEFAULT_SCHEDULE_START_ID: i64 = 1;
const DEFAULT_YEAR: i64 = 2026;

#[derive(Clone)]
pub struct WatchStore {
    path: PathBuf,
}

pub struct WatchSummary {
    pub key: String,
    pub meet_name: String,
    pub page_url: String,
}

pub struct NewWatch {
    pub key: String,
    pub meet_name: String,
    pub page_url: String,
    pub start_list_url: Option<String>,
    pub schedule_url: Option<String>,
}

impl WatchStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> Result<Vec<Value>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("could not read watches file: {e}"))?;
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("watches file is not valid JSON: {e}"))?;
        match value {
            Value::Array(items) => Ok(items),
            _ => Err("watches file must contain a JSON array".to_string()),
        }
    }

    fn save(&self, watches: &[Value]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create watches directory: {e}"))?;
        }
        let body = serde_json::to_string_pretty(&watches)
            .map_err(|e| format!("could not serialize watches: {e}"))?;
        // Write to a temp file in the same dir, then rename for an atomic swap
        // so a concurrent pipeline read never sees a half-written file.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, format!("{body}\n"))
            .map_err(|e| format!("could not write watches file: {e}"))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("could not replace watches file: {e}"))?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<WatchSummary>, String> {
        let watches = self.load()?;
        Ok(watches
            .iter()
            .map(|w| WatchSummary {
                key: string_field(w, "key"),
                meet_name: string_field(w, "meet_name"),
                page_url: string_field(w, "page_url"),
            })
            .collect())
    }

    pub fn add(&self, spec: NewWatch) -> Result<(), String> {
        validate_key(&spec.key)?;
        require_http_url("page url", Some(&spec.page_url))?;
        require_http_url("start-list url", spec.start_list_url.as_deref())?;
        require_http_url("schedule url", spec.schedule_url.as_deref())?;

        let mut watches = self.load()?;
        if watches
            .iter()
            .any(|w| string_field(w, "key").eq_ignore_ascii_case(&spec.key))
        {
            return Err(format!("a watch with key `{}` already exists", spec.key));
        }

        watches.push(json!({
            "key": spec.key,
            "meet_name": spec.meet_name,
            "page_url": spec.page_url,
            "start_list_url": spec.start_list_url,
            "schedule_url": spec.schedule_url,
            "source_format": DEFAULT_SOURCE_FORMAT,
            "start_member_id": DEFAULT_START_MEMBER_ID,
            "schedule_start_id": DEFAULT_SCHEDULE_START_ID,
            "default_year": DEFAULT_YEAR,
        }));
        self.save(&watches)
    }

    /// Remove a watch by key. Returns `Ok(true)` when one was removed.
    pub fn delete(&self, key: &str) -> Result<bool, String> {
        let mut watches = self.load()?;
        let before = watches.len();
        watches.retain(|w| !string_field(w, "key").eq_ignore_ascii_case(key));
        let removed = watches.len() != before;
        if removed {
            self.save(&watches)?;
        }
        Ok(removed)
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Validate an optional URL field. `None` passes (the field is absent).
fn require_http_url(label: &str, url: Option<&str>) -> Result<(), String> {
    match url {
        Some(u) if !is_http_url(u) => Err(format!("{label} must start with http(s): `{u}`")),
        _ => Ok(()),
    }
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("key must not be empty".to_string());
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "key `{key}` may only contain letters, numbers, `-` and `_`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (WatchStore, tempdir::Guard) {
        let dir = tempdir::Guard::new();
        (WatchStore::new(dir.path().join("watches.json")), dir)
    }

    #[test]
    fn add_list_delete_roundtrip() {
        let (s, _g) = store();
        assert!(s.list().unwrap().is_empty());

        s.add(NewWatch {
            key: "2026-nationals".into(),
            meet_name: "2026 Nationals".into(),
            page_url: "https://example.com/nats".into(),
            start_list_url: None,
            schedule_url: None,
        })
        .unwrap();

        let list = s.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].key, "2026-nationals");

        // duplicate rejected
        assert!(
            s.add(NewWatch {
                key: "2026-nationals".into(),
                meet_name: "dup".into(),
                page_url: "https://example.com/x".into(),
                start_list_url: None,
                schedule_url: None,
            })
            .is_err()
        );

        assert!(s.delete("2026-NATIONALS").unwrap()); // case-insensitive
        assert!(s.list().unwrap().is_empty());
        assert!(!s.delete("missing").unwrap());
    }

    #[test]
    fn rejects_bad_key_and_url() {
        let (s, _g) = store();
        assert!(
            s.add(NewWatch {
                key: "bad key".into(),
                meet_name: "x".into(),
                page_url: "https://e.com".into(),
                start_list_url: None,
                schedule_url: None,
            })
            .is_err()
        );
        assert!(
            s.add(NewWatch {
                key: "ok".into(),
                meet_name: "x".into(),
                page_url: "ftp://e.com".into(),
                start_list_url: None,
                schedule_url: None,
            })
            .is_err()
        );
    }

    /// Tiny self-cleaning temp dir helper (avoids adding a dev-dependency).
    mod tempdir {
        use std::path::{Path, PathBuf};
        pub struct Guard {
            path: PathBuf,
        }
        impl Guard {
            pub fn new() -> Self {
                let mut p = std::env::temp_dir();
                let n = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                p.push(format!("meetcal-watch-test-{n}"));
                std::fs::create_dir_all(&p).unwrap();
                Guard { path: p }
            }
            pub fn path(&self) -> &Path {
                &self.path
            }
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }
}
