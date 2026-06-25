//! Scraper-facing routes. Currently just the Slack slash-command endpoint that
//! lets the meet-automation channel manage which meet pages are watched.
//!
//! This is the app's only mutating surface: it edits `watches.json` (consumed
//! by `scrapers/usaw/meet_automation`), guarded by Slack signing-secret
//! verification and an optional channel/user allowlist. It touches no database.

pub mod signature;
pub mod slack_commands;
pub mod watches;

use std::path::PathBuf;

use watches::WatchStore;

/// Runtime Slack configuration, read from the environment at startup. The
/// endpoint is disabled (returns 503) unless `signing_secret` is set.
#[derive(Clone)]
pub struct SlackConfig {
    pub signing_secret: String,
    /// If non-empty, commands are only honored from this Slack channel id.
    pub channel_id: String,
    /// If non-empty, commands are only honored from these Slack user ids.
    pub allowed_users: Vec<String>,
    watches_path: PathBuf,
}

impl SlackConfig {
    pub fn from_env() -> Self {
        let default_watches_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scrapers/usaw/meet_automation/watches.json");
        let watches_path = std::env::var("MEET_AUTOMATION_WATCHES_PATH")
            .map(PathBuf::from)
            .unwrap_or(default_watches_path);

        let allowed_users = std::env::var("MEET_AUTOMATION_SLACK_ALLOWED_USERS")
            .unwrap_or_default()
            .split([',', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        Self {
            signing_secret: std::env::var("SLACK_SIGNING_SECRET").unwrap_or_default(),
            channel_id: std::env::var("SLACK_MEET_AUTOMATION_CHANNEL").unwrap_or_default(),
            allowed_users,
            watches_path,
        }
    }

    pub fn enabled(&self) -> bool {
        !self.signing_secret.is_empty()
    }

    pub fn store(&self) -> WatchStore {
        WatchStore::new(self.watches_path.clone())
    }
}
