//! Slack slash-command handler for managing scraper lists.
//!
//! One route backs all commands across all channels; it verifies the Slack
//! signature, routes the request to the list bound to the originating channel
//! (meet watches or entry targets), and dispatches `list` / `add` / `delete`.
//! The action is taken from the command name (e.g. `/meet-add`, `/entries-add`)
//! or the first word of the command text. Replies are ephemeral.

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::store::{JsonListStore, require_http_url, validate_slug};
use super::{ListKind, signature};
use crate::AppState;

const WATCHES_USAGE: &str = "*Meet watches*\n\
    • `list` — show watched meet pages\n\
    • `add <key> | <meet name> | <page url> [| <start-list url> | <schedule url>]`\n\
    • `delete <key>`";

const ENTRIES_USAGE: &str = "*Entry targets*\n\
    • `list` — show meet entry URLs being scraped\n\
    • `add <label> | <entries url>`\n\
    • `delete <label>`";

const GENERIC_USAGE: &str = "Use a meet or entries command, e.g. `/meet-list`, \
    `/meet-add`, `/entries-list`, `/entries-add`.";

#[derive(Deserialize, Default)]
struct SlackCommand {
    #[serde(default)]
    command: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    channel_id: String,
    #[serde(default)]
    user_id: String,
}

enum Action {
    List,
    Add,
    Delete,
    Help,
}

pub async fn slack_commands(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let cfg = &state.slack;
    if !cfg.enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Slack scraper commands are not configured",
        )
            .into_response();
    }
    if let Some(resp) = signature::require_valid(&cfg.signing_secret, &headers, &body) {
        return resp;
    }

    let Ok(cmd) = serde_urlencoded::from_bytes::<SlackCommand>(&body) else {
        return ephemeral("could not parse slash command payload");
    };

    if !cfg.channel_allowed(&cmd.channel_id) {
        return ephemeral("These commands aren't enabled in this channel.");
    }
    if !cfg.user_allowed(&cmd.user_id) {
        return ephemeral("You're not authorized to manage scraper lists.");
    }

    // The command name decides which list (one channel can host both).
    let Some(kind) = route_kind(&cmd.command, &cmd.text) else {
        return ephemeral(GENERIC_USAGE);
    };
    let (action, args) = parse_action(&cmd.command, &cmd.text);
    let store = cfg.store_for(kind);
    let text = match kind {
        ListKind::Watches => watches_reply(action, &args, &store),
        ListKind::Entries => entries_reply(action, &args, &store),
    };
    ephemeral(&text)
}

/// Decide which list a command targets from its name (and, as a fallback, the
/// first word of its text): `/meet-*` / `/watch-*` → watches, `/entries-*` /
/// `/entry-*` → entries.
fn route_kind(command: &str, text: &str) -> Option<ListKind> {
    let name = command.trim_start_matches('/').to_ascii_lowercase();
    if name.contains("entr") {
        return Some(ListKind::Entries);
    }
    if name.contains("watch") || name.contains("meet") {
        return Some(ListKind::Watches);
    }
    // Generic command (e.g. `/scraper entries list`): look at the first word.
    match text.split_whitespace().next().unwrap_or("") {
        "entries" | "entry" => Some(ListKind::Entries),
        "meet" | "meets" | "watch" | "watches" => Some(ListKind::Watches),
        _ => None,
    }
}

// --- meet watches ---------------------------------------------------------
fn watches_reply(action: Action, args: &str, store: &JsonListStore) -> String {
    match action {
        Action::List => match store.items() {
            Ok(items) if items.is_empty() => "No meet pages are being watched.".to_string(),
            Ok(items) => {
                let mut out = format!("*Watching {} meet page(s):*", items.len());
                for w in items {
                    out.push_str(&format!(
                        "\n• `{}` — {}\n   {}",
                        field(&w, "key"),
                        field(&w, "meet_name"),
                        field(&w, "page_url"),
                    ));
                }
                out
            }
            Err(msg) => format!(":warning: {msg}"),
        },
        Action::Add => match build_watch(args) {
            Ok(obj) => {
                let key = field(&obj, "key");
                match store.add(obj) {
                    Ok(()) => format!(":white_check_mark: Added watch `{key}`."),
                    Err(msg) => format!(":warning: {msg}"),
                }
            }
            Err(msg) => format!(":warning: {msg}\n\n{WATCHES_USAGE}"),
        },
        Action::Delete => delete_reply(args, store, "watch", WATCHES_USAGE),
        Action::Help => WATCHES_USAGE.to_string(),
    }
}

fn build_watch(args: &str) -> Result<Value, String> {
    let fields: Vec<String> = args.split('|').map(|s| s.trim().to_string()).collect();
    let get = |i: usize| fields.get(i).filter(|s| !s.is_empty()).cloned();

    let key = get(0).ok_or("missing <key>")?;
    let meet_name = get(1).ok_or("missing <meet name>")?;
    let page_url = get(2).ok_or("missing <page url>")?;
    let start_list_url = get(3);
    let schedule_url = get(4);

    validate_slug("key", &key)?;
    require_http_url("page url", Some(&page_url))?;
    require_http_url("start-list url", start_list_url.as_deref())?;
    require_http_url("schedule url", schedule_url.as_deref())?;

    Ok(json!({
        "key": key,
        "meet_name": meet_name,
        "page_url": page_url,
        "start_list_url": start_list_url,
        "schedule_url": schedule_url,
        "source_format": "auto",
        "start_member_id": 3100,
        "schedule_start_id": 1,
        "default_year": 2026,
    }))
}

// --- entry targets --------------------------------------------------------
fn entries_reply(action: Action, args: &str, store: &JsonListStore) -> String {
    match action {
        Action::List => match store.items() {
            Ok(items) if items.is_empty() => "No meet entries are being scraped.".to_string(),
            Ok(items) => {
                let mut out = format!("*Scraping entries for {} target(s):*", items.len());
                for e in items {
                    out.push_str(&format!("\n• `{}` — {}", field(&e, "label"), field(&e, "url")));
                }
                out
            }
            Err(msg) => format!(":warning: {msg}"),
        },
        Action::Add => match build_entry(args) {
            Ok(obj) => {
                let label = field(&obj, "label");
                match store.add(obj) {
                    Ok(()) => format!(":white_check_mark: Added entry target `{label}`."),
                    Err(msg) => format!(":warning: {msg}"),
                }
            }
            Err(msg) => format!(":warning: {msg}\n\n{ENTRIES_USAGE}"),
        },
        Action::Delete => delete_reply(args, store, "entry target", ENTRIES_USAGE),
        Action::Help => ENTRIES_USAGE.to_string(),
    }
}

fn build_entry(args: &str) -> Result<Value, String> {
    let fields: Vec<String> = args.split('|').map(|s| s.trim().to_string()).collect();
    let get = |i: usize| fields.get(i).filter(|s| !s.is_empty()).cloned();

    let label = get(0).ok_or("missing <label>")?;
    let url = get(1).ok_or("missing <entries url>")?;
    require_http_url("entries url", Some(&url))?;

    Ok(json!({ "label": label, "url": url }))
}

// --- shared ---------------------------------------------------------------
fn delete_reply(args: &str, store: &JsonListStore, noun: &str, usage: &str) -> String {
    let key = args.trim();
    if key.is_empty() {
        return format!(":warning: usage: `delete <key>`\n\n{usage}");
    }
    match store.delete(key) {
        Ok(true) => format!(":white_check_mark: Deleted {noun} `{key}`."),
        Ok(false) => format!(":mag: No {noun} matching `{key}`."),
        Err(msg) => format!(":warning: {msg}"),
    }
}

fn field(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn ephemeral(text: &str) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "response_type": "ephemeral", "text": text })),
    )
        .into_response()
}

fn parse_action(command: &str, text: &str) -> (Action, String) {
    let name = command.trim_start_matches('/').to_ascii_lowercase();
    if name.ends_with("list") || name.ends_with("-ls") || name == "ls" {
        return (Action::List, text.trim().to_string());
    }
    if name.ends_with("add") {
        return (Action::Add, text.trim().to_string());
    }
    if name.ends_with("delete")
        || name.ends_with("remove")
        || name.ends_with("-del")
        || name.ends_with("-rm")
    {
        return (Action::Delete, text.trim().to_string());
    }

    // Generic command (e.g. `/meet add …`): dispatch on the first word.
    let mut parts = text.trim().splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("").to_ascii_lowercase();
    let rest = parts.next().unwrap_or("").trim().to_string();
    match first.as_str() {
        "add" => (Action::Add, rest),
        "delete" | "remove" | "del" | "rm" => (Action::Delete, rest),
        "list" | "ls" | "" => (Action::List, rest),
        "help" => (Action::Help, rest),
        _ => (Action::Help, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_kind_by_command_name() {
        assert_eq!(route_kind("/meet-list", ""), Some(ListKind::Watches));
        assert_eq!(route_kind("/meet-add", "a|b|c"), Some(ListKind::Watches));
        assert_eq!(route_kind("/entries-add", "a|b"), Some(ListKind::Entries));
        assert_eq!(route_kind("/entry-list", ""), Some(ListKind::Entries));
        assert_eq!(route_kind("/watch-delete", "k"), Some(ListKind::Watches));
        // Generic command falls back to the first word of the text.
        assert_eq!(route_kind("/scraper", "entries list"), Some(ListKind::Entries));
        assert_eq!(route_kind("/scraper", "meet list"), Some(ListKind::Watches));
        assert_eq!(route_kind("/scraper", "huh"), None);
    }

    #[test]
    fn dispatch_by_command_name() {
        assert!(matches!(parse_action("/meet-list", ""), (Action::List, _)));
        assert!(matches!(parse_action("/entries-add", "a|b"), (Action::Add, _)));
        assert!(matches!(parse_action("/meet-delete", "k"), (Action::Delete, _)));
    }

    #[test]
    fn dispatch_by_text_word() {
        let (a, rest) = parse_action("/meet", "add k | n | u");
        assert!(matches!(a, Action::Add));
        assert_eq!(rest, "k | n | u");
        assert!(matches!(parse_action("/meet", ""), (Action::List, _)));
    }

    #[test]
    fn build_watch_parsing() {
        let w = build_watch("2026-nats | 2026 Nationals | https://e.com/p").unwrap();
        assert_eq!(field(&w, "key"), "2026-nats");
        assert_eq!(field(&w, "page_url"), "https://e.com/p");
        assert_eq!(w["start_member_id"], 3100);
        assert!(w["start_list_url"].is_null());

        let full = build_watch("k | n | https://e.com/p | https://e.com/s | https://e.com/c").unwrap();
        assert_eq!(field(&full, "start_list_url"), "https://e.com/s");

        assert!(build_watch("only-key").is_err());
        assert!(build_watch("bad key | n | https://e.com").is_err());
    }

    #[test]
    fn build_entry_parsing() {
        let e = build_entry("Masters Nats | https://usaweightlifting.sport80.com/public/events/1/entries/2").unwrap();
        assert_eq!(field(&e, "label"), "Masters Nats");
        assert!(build_entry("no url").is_err());
        assert!(build_entry("label | ftp://nope").is_err());
    }
}
