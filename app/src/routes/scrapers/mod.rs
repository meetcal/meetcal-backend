//! Scraper-facing routes: Slack slash commands and interactive buttons that let
//! dedicated Slack channels manage scraper config — which meet pages are watched
//! and which meet entries are scraped — plus approve/reject staged meet uploads.
//!
//! These are the app's only mutating surfaces. They edit JSON list files on the
//! server's filesystem (`watches.json`, `entries_targets.json`) and drop
//! approval decision files for the Python pipeline; they touch no database. All
//! requests are Slack-signature verified and routed by channel. Because the
//! files live on disk next to the cron jobs that read them, edits take effect on
//! the running server with no redeploy or git pull.

pub mod interactions;
pub mod signature;
pub mod slack_commands;
pub mod store;

use std::path::PathBuf;

use store::JsonListStore;

/// Which managed list a channel maps to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListKind {
    /// Meet pages watched by the meet-automation pipeline (`watches.json`).
    Watches,
    /// Meet entry URLs scraped by the entries job (`entries_targets.json`).
    Entries,
}

impl ListKind {
    /// The field used as each item's unique key.
    pub fn key_field(self) -> &'static str {
        match self {
            ListKind::Watches => "key",
            ListKind::Entries => "label",
        }
    }
}

/// One Slack channel bound to one managed list file.
#[derive(Clone)]
pub struct Surface {
    /// Empty means "any channel" (single-list back-compat).
    pub channel_id: String,
    pub kind: ListKind,
    pub path: PathBuf,
}

impl Surface {
    pub fn store(&self) -> JsonListStore {
        JsonListStore::new(self.path.clone(), self.kind.key_field())
    }
}

/// Runtime Slack configuration, read from the environment at startup. Both
/// endpoints are disabled (503) unless `signing_secret` is set.
#[derive(Clone)]
pub struct SlackConfig {
    pub signing_secret: String,
    pub allowed_users: Vec<String>,
    pub surfaces: Vec<Surface>,
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
        let state_dir = env_path(
            "MEET_AUTOMATION_STATE_DIR",
            app_dir.join("../scrapers/usaw/meet_automation/state"),
        );

        let watches_channel = env_str("SLACK_MEET_AUTOMATION_CHANNEL");
        let entries_channel = env_str("SLACK_ENTRIES_CHANNEL");

        // Watches always present (channel may be empty = any channel, for the
        // original single-channel setup). Entries only when its channel is set.
        let mut surfaces = vec![Surface {
            channel_id: watches_channel,
            kind: ListKind::Watches,
            path: watches_path,
        }];
        if !entries_channel.is_empty() {
            surfaces.push(Surface {
                channel_id: entries_channel,
                kind: ListKind::Entries,
                path: entries_path,
            });
        }

        Self {
            signing_secret: env_str("SLACK_SIGNING_SECRET"),
            allowed_users: env_str("MEET_AUTOMATION_SLACK_ALLOWED_USERS")
                .split([',', ' '])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            surfaces,
            state_dir,
        }
    }

    pub fn enabled(&self) -> bool {
        !self.signing_secret.is_empty()
    }

    /// Resolve a channel id to its managed list: an exact channel match wins,
    /// otherwise a wildcard (empty-channel) surface is used.
    pub fn resolve(&self, channel_id: &str) -> Option<&Surface> {
        self.surfaces
            .iter()
            .find(|s| !s.channel_id.is_empty() && s.channel_id == channel_id)
            .or_else(|| self.surfaces.iter().find(|s| s.channel_id.is_empty()))
    }

    pub fn user_allowed(&self, user_id: &str) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.iter().any(|u| u == user_id)
    }

    pub fn decisions_dir(&self) -> PathBuf {
        self.state_dir.join("decisions")
    }
}

fn env_str(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn env_path(name: &str, default: PathBuf) -> PathBuf {
    std::env::var(name).map(PathBuf::from).unwrap_or(default)
}
