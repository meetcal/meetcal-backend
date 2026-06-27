//! Scraper-facing routes: Slack slash commands and interactive buttons that let
//! a Slack workspace manage scraper config — which meet pages are watched and
//! which meet entries are scraped — plus approve/reject staged meet uploads.
//!
//! These are the app's only mutating surfaces. They edit JSON list files on the
//! server's filesystem (`watches.json`, `entries_targets.json`) and drop
//! approval decision files for the Python pipeline; they touch no database. All
//! requests are Slack-signature verified. Because the files live on disk next to
//! the cron jobs that read them, edits take effect on the running server with no
//! redeploy or git pull.
//!
//! Commands are routed to a list by the **command name** (`/meet-*` vs
//! `/entries-*`), so both lists can live in one Slack channel or be split across
//! channels — channels act only as an optional allowlist.

pub mod interactions;
pub mod signature;
pub mod slack_commands;
pub mod store;

use std::path::PathBuf;

use store::{ListFormat, ListStore};

/// Which managed list a command targets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListKind {
    /// Meet pages watched by the meet-automation pipeline (`watches.json`).
    Watches,
    /// Meet entry URLs scraped by the entries job (`entries_targets.json`).
    Entries,
    /// Pages watched for changes by urlwatch (`urls.yaml`, multi-doc YAML).
    UrlWatch,
}

impl ListKind {
    /// The field used as each item's unique key.
    pub fn key_field(self) -> &'static str {
        match self {
            ListKind::Watches => "key",
            ListKind::Entries => "label",
            ListKind::UrlWatch => "name",
        }
    }

    /// How the list is encoded on disk.
    pub fn format(self) -> ListFormat {
        match self {
            ListKind::Watches | ListKind::Entries => ListFormat::Json,
            ListKind::UrlWatch => ListFormat::Yaml,
        }
    }
}

/// Runtime Slack configuration, read from the environment at startup. Both
/// endpoints are disabled (503) unless `signing_secret` is set.
#[derive(Clone)]
pub struct SlackConfig {
    pub signing_secret: String,
    pub allowed_users: Vec<String>,
    /// Channels permitted to use the commands. Empty = any channel.
    pub allowed_channels: Vec<String>,
    watches_path: PathBuf,
    entries_path: PathBuf,
    urlwatch_path: PathBuf,
    /// Where staged runs + approval decisions live (shared with the Python
    /// pipeline via `MEET_AUTOMATION_STATE_DIR`).
    pub state_dir: PathBuf,
}

impl SlackConfig {
    pub fn from_env() -> Self {
        let app_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let watches_path = env_path(
            "MEET_AUTOMATION_WATCHES_PATH",
            app_dir.join("../scrapers/usaw/meet_automation/watches.json"),
        );
        let entries_path = env_path(
            "ENTRIES_TARGETS_PATH",
            app_dir.join("../scrapers/usaw/entry_scraper/entries_targets.json"),
        );
        let urlwatch_path = env_path(
            "URLWATCH_URLS_PATH",
            app_dir.join("../scrapers/urlwatch/urls.yaml"),
        );
        let state_dir = env_path(
            "MEET_AUTOMATION_STATE_DIR",
            app_dir.join("../scrapers/usaw/meet_automation/state"),
        );

        // Either or both channels may be set; they're merged into one allowlist.
        // Using the same value for both is fine (one shared channel).
        let mut allowed_channels = Vec::new();
        for var in ["SLACK_MEET_AUTOMATION_CHANNEL", "SLACK_ENTRIES_CHANNEL"] {
            let value = env_str(var);
            if !value.is_empty() && !allowed_channels.contains(&value) {
                allowed_channels.push(value);
            }
        }

        Self {
            signing_secret: env_str("SLACK_SIGNING_SECRET"),
            allowed_users: split_list(&env_str("MEET_AUTOMATION_SLACK_ALLOWED_USERS")),
            allowed_channels,
            watches_path,
            entries_path,
            urlwatch_path,
            state_dir,
        }
    }

    pub fn enabled(&self) -> bool {
        !self.signing_secret.is_empty()
    }

    pub fn channel_allowed(&self, channel_id: &str) -> bool {
        self.allowed_channels.is_empty() || self.allowed_channels.iter().any(|c| c == channel_id)
    }

    pub fn user_allowed(&self, user_id: &str) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.iter().any(|u| u == user_id)
    }

    pub fn store_for(&self, kind: ListKind) -> ListStore {
        let path = match kind {
            ListKind::Watches => self.watches_path.clone(),
            ListKind::Entries => self.entries_path.clone(),
            ListKind::UrlWatch => self.urlwatch_path.clone(),
        };
        ListStore::new(path, kind.key_field(), kind.format())
    }

    pub fn decisions_dir(&self) -> PathBuf {
        self.state_dir.join("decisions")
    }

    /// Where `/meet-run` drops "run this watch now" requests for the Python
    /// pipeline's `run --requested` cron to drain (shared `state_dir`).
    pub fn run_requests_dir(&self) -> PathBuf {
        self.state_dir.join("run_requests")
    }
}

/// Unix epoch seconds. Used to stamp decision / run-request files dropped on the
/// shared filesystem; informational only, so we avoid pulling in a date crate.
pub(crate) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn env_str(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn env_path(name: &str, default: PathBuf) -> PathBuf {
    std::env::var(name).map(PathBuf::from).unwrap_or(default)
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
