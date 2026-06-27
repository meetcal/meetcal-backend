//! Slack slash-command handler for managing scraper lists.
//!
//! One route backs all commands across all channels; it verifies the Slack
//! signature, routes the request to the list named by the command (meet
//! watches, entry targets, or urlwatch pages), and dispatches
//! `list` / `add` / `delete` (plus `run` for meet watches). The action is taken
//! from the command name (e.g. `/meet-add`, `/entries-add`, `/url-add`) or the
//! first word of the command text. Replies are ephemeral.

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::store::{ListStore, require_http_url, validate_slug};
use super::{ListKind, now_unix_secs, signature};
use crate::AppState;

const WATCHES_USAGE: &str = "*Meet watches*\n\
    • `list` — show watched meet pages\n\
    • `add <key> | <meet name> | <page url> [| <start-list url> | <schedule url>]`\n\
    • `delete <key>`\n\
    • `run <key>` (or `/meet-run <key>`) — scrape + validate + stage it now";

const RUN_USAGE: &str = "*Run a meet now*\n\
    • `/meet-run <key>` — scrape + validate + stage one watched meet for review\n\
    • `/meet-run all` — do it for every watched meet\n\
    The pipeline posts a review here with Approve / Reject buttons; nothing is \
    written to the database until you approve.";

const ENTRIES_USAGE: &str = "*Entry targets*\n\
    • `list` — show meet entry URLs being scraped\n\
    • `add <label> | <entries url>`\n\
    • `delete <label>`";

const URLWATCH_USAGE: &str = "*URL watches*\n\
    • `list` — show pages watched for changes\n\
    • `add <name> | <url> [| <css selector>]` — defaults to full-page text; a \
    CSS selector narrows what's diffed\n\
    • `delete <name>`";

const GENERIC_USAGE: &str = "Use a meet, entries, or url command, e.g. \
    `/meet-list`, `/meet-add`, `/meet-run`, `/entries-list`, `/entries-add`, \
    `/url-list`, `/url-add`.";

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
    /// Trigger a pipeline run now (meet watches only).
    Run,
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
    let text = match (kind, action) {
        // Running the pipeline is a meet-watch concept; it needs the shared
        // state dir, so it gets `cfg` rather than just the list store.
        (ListKind::Watches, Action::Run) => run_reply(cfg, &args, &cmd.user_id),
        (_, Action::Run) => {
            ":information_source: `run` applies to meet watches. Try `/meet-run <key>`.".to_string()
        }
        (ListKind::Watches, action) => watches_reply(action, &args, &cfg.store_for(kind)),
        (ListKind::Entries, action) => entries_reply(action, &args, &cfg.store_for(kind)),
        (ListKind::UrlWatch, action) => urlwatch_reply(action, &args, &cfg.store_for(kind)),
    };
    ephemeral(&text)
}

/// Decide which list a command targets from its name (and, as a fallback, the
/// first word of its text): `/meet-*` / `/watch-*` → watches, `/entries-*` /
/// `/entry-*` → entries, `/url-*` → urlwatch.
fn route_kind(command: &str, text: &str) -> Option<ListKind> {
    let name = command.trim_start_matches('/').to_ascii_lowercase();
    if name.contains("entr") {
        return Some(ListKind::Entries);
    }
    // Check `url` before `watch`: `/urlwatch-*` contains both, and it's urlwatch.
    if name.contains("url") {
        return Some(ListKind::UrlWatch);
    }
    if name.contains("watch") || name.contains("meet") {
        return Some(ListKind::Watches);
    }
    // Generic command (e.g. `/scraper entries list`): look at the first word.
    match text.split_whitespace().next().unwrap_or("") {
        "entries" | "entry" => Some(ListKind::Entries),
        "url" | "urls" | "urlwatch" => Some(ListKind::UrlWatch),
        "meet" | "meets" | "watch" | "watches" => Some(ListKind::Watches),
        _ => None,
    }
}

// --- meet watches ---------------------------------------------------------
fn watches_reply(action: Action, args: &str, store: &ListStore) -> String {
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
        // Routed separately by `slack_commands` (it needs the shared state dir).
        Action::Run => RUN_USAGE.to_string(),
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

// --- run now --------------------------------------------------------------
/// Handle `/meet-run <key>` / `/meet-run all`. The API can't run the Python
/// pipeline itself (no DB creds, and Slack's 3s slash timeout can't wait for a
/// scrape), so it drops a request file the `run --requested` cron drains —
/// mirroring the Approve/Reject decision-file handshake.
fn run_reply(cfg: &super::SlackConfig, args: &str, user_id: &str) -> String {
    let target = match parse_run_target(args) {
        Ok(t) => t,
        Err(msg) => return format!(":warning: {msg}\n\n{RUN_USAGE}"),
    };

    // For a specific key, confirm it's actually being watched before queueing.
    if let Some(key) = &target {
        let store = cfg.store_for(ListKind::Watches);
        match store.items() {
            Ok(items) if !items.iter().any(|i| store.key_of(i).eq_ignore_ascii_case(key)) => {
                return format!(
                    ":mag: No meet watch matching `{key}`. Add it with `/meet-add` first, \
                     or run every watch with `/meet-run all`."
                );
            }
            Err(msg) => return format!(":warning: {msg}"),
            _ => {}
        }
    }

    match queue_run(cfg, target.as_deref(), user_id) {
        Ok(()) => {
            let what = match &target {
                Some(key) => format!("`{key}`"),
                None => "all watched meets".to_string(),
            };
            format!(
                ":hourglass_flowing_sand: Queued a pipeline run for {what}. It'll scrape, \
                 validate, and post a review here with Approve / Reject buttons shortly."
            )
        }
        Err(msg) => format!(":warning: Could not queue run: {msg}"),
    }
}

/// Parse the `/meet-run` argument. Empty or `all` ⇒ every watch (`Ok(None)`);
/// otherwise the single watch key.
fn parse_run_target(args: &str) -> Result<Option<String>, String> {
    let first = args.split_whitespace().next().unwrap_or("");
    if first.is_empty() || first.eq_ignore_ascii_case("all") {
        return Ok(None);
    }
    validate_slug("key", first)?;
    Ok(Some(first.to_string()))
}

/// Atomically drop a run-request file for the pipeline to pick up. `None` keys
/// off the `__all__` sentinel; a specific key keys off itself, so re-running the
/// same watch before the cron drains it just refreshes one file.
fn queue_run(cfg: &super::SlackConfig, key: Option<&str>, user_id: &str) -> Result<(), String> {
    let dir = cfg.run_requests_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create dir: {e}"))?;
    let body = run_request_body(key, user_id);
    let path = dir.join(run_request_filename(key));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&body).map_err(|e| e.to_string())?)
        .map_err(|e| format!("write: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

fn run_request_filename(key: Option<&str>) -> String {
    match key {
        Some(key) => format!("{key}.json"),
        None => "__all__.json".to_string(),
    }
}

fn run_request_body(key: Option<&str>, user_id: &str) -> Value {
    json!({
        "key": key,
        "all": key.is_none(),
        // A manual trigger should always produce a staged run to review, even if
        // the source PDFs haven't changed since the last automatic run.
        "force": true,
        "requested_by_id": user_id,
        "requested_at_unix": now_unix_secs(),
    })
}

// --- entry targets --------------------------------------------------------
fn entries_reply(action: Action, args: &str, store: &ListStore) -> String {
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
        Action::Run => {
            ":information_source: `run` applies to meet watches. Try `/meet-run <key>`.".to_string()
        }
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

// --- url watches ----------------------------------------------------------
fn urlwatch_reply(action: Action, args: &str, store: &ListStore) -> String {
    match action {
        Action::List => match store.items() {
            Ok(items) if items.is_empty() => "No pages are being watched for changes.".to_string(),
            Ok(items) => {
                let mut out = format!("*Watching {} page(s) for changes:*", items.len());
                for u in items {
                    out.push_str(&format!("\n• `{}` — {}", field(&u, "name"), field(&u, "url")));
                }
                out
            }
            Err(msg) => format!(":warning: {msg}"),
        },
        Action::Add => match build_urlwatch(args) {
            Ok(obj) => {
                let name = field(&obj, "name");
                match store.add(obj) {
                    Ok(()) => format!(":white_check_mark: Now watching `{name}` for changes."),
                    Err(msg) => format!(":warning: {msg}"),
                }
            }
            Err(msg) => format!(":warning: {msg}\n\n{URLWATCH_USAGE}"),
        },
        Action::Delete => delete_reply(args, store, "url watch", URLWATCH_USAGE),
        Action::Run => {
            ":information_source: `run` applies to meet watches. Try `/meet-run <key>`.".to_string()
        }
        Action::Help => URLWATCH_USAGE.to_string(),
    }
}

/// Build one urlwatch job from `name | url [| css selector]`. The `filter`
/// chain mirrors the hand-written `urls.yaml`: an optional CSS narrowing, then
/// `html2text` so diffs are over rendered text rather than raw HTML.
fn build_urlwatch(args: &str) -> Result<Value, String> {
    let fields: Vec<String> = args.split('|').map(|s| s.trim().to_string()).collect();
    let get = |i: usize| fields.get(i).filter(|s| !s.is_empty()).cloned();

    let name = get(0).ok_or("missing <name>")?;
    let url = get(1).ok_or("missing <url>")?;
    require_http_url("url", Some(&url))?;

    let mut filter: Vec<Value> = Vec::new();
    if let Some(css) = get(2) {
        filter.push(json!({ "css": css }));
    }
    filter.push(json!("html2text"));

    Ok(json!({ "name": name, "url": url, "filter": filter }))
}

// --- shared ---------------------------------------------------------------
fn delete_reply(args: &str, store: &ListStore, noun: &str, usage: &str) -> String {
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
    if name.ends_with("run") {
        return (Action::Run, text.trim().to_string());
    }

    // Generic command (e.g. `/meet add …`): dispatch on the first word.
    let mut parts = text.trim().splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("").to_ascii_lowercase();
    let rest = parts.next().unwrap_or("").trim().to_string();
    match first.as_str() {
        "add" => (Action::Add, rest),
        "delete" | "remove" | "del" | "rm" => (Action::Delete, rest),
        "run" => (Action::Run, rest),
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
        assert_eq!(route_kind("/url-list", ""), Some(ListKind::UrlWatch));
        assert_eq!(route_kind("/url-add", "n|u"), Some(ListKind::UrlWatch));
        assert_eq!(route_kind("/url-delete", "n"), Some(ListKind::UrlWatch));
        // `/urlwatch-*` carries both "url" and "watch"; url wins (it's urlwatch).
        assert_eq!(route_kind("/urlwatch-add", "n|u"), Some(ListKind::UrlWatch));
        // Generic command falls back to the first word of the text.
        assert_eq!(route_kind("/scraper", "entries list"), Some(ListKind::Entries));
        assert_eq!(route_kind("/scraper", "meet list"), Some(ListKind::Watches));
        assert_eq!(route_kind("/scraper", "url add n | u"), Some(ListKind::UrlWatch));
        assert_eq!(route_kind("/scraper", "huh"), None);
    }

    #[test]
    fn dispatch_by_command_name() {
        assert!(matches!(parse_action("/meet-list", ""), (Action::List, _)));
        assert!(matches!(parse_action("/entries-add", "a|b"), (Action::Add, _)));
        assert!(matches!(parse_action("/meet-delete", "k"), (Action::Delete, _)));
        assert!(matches!(parse_action("/meet-run", "2026-nats"), (Action::Run, _)));
    }

    #[test]
    fn run_routes_to_watches() {
        // `/meet-run` carries "meet", so it targets the watches list.
        assert_eq!(route_kind("/meet-run", "2026-nats"), Some(ListKind::Watches));
    }

    #[test]
    fn dispatch_run_by_text_word() {
        let (a, rest) = parse_action("/meet", "run 2026-nats");
        assert!(matches!(a, Action::Run));
        assert_eq!(rest, "2026-nats");
    }

    #[test]
    fn run_target_parsing() {
        assert_eq!(parse_run_target("").unwrap(), None);
        assert_eq!(parse_run_target("   ").unwrap(), None);
        assert_eq!(parse_run_target("all").unwrap(), None);
        assert_eq!(parse_run_target("ALL").unwrap(), None);
        assert_eq!(parse_run_target("2026-nats").unwrap(), Some("2026-nats".to_string()));
        // Only the first token is taken as the key.
        assert_eq!(parse_run_target("2026-nats now").unwrap(), Some("2026-nats".to_string()));
        assert!(parse_run_target("bad/key").is_err());
    }

    #[test]
    fn run_request_filename_and_body() {
        assert_eq!(run_request_filename(Some("2026-nats")), "2026-nats.json");
        assert_eq!(run_request_filename(None), "__all__.json");

        let one = run_request_body(Some("2026-nats"), "U1");
        assert_eq!(field(&one, "key"), "2026-nats");
        assert_eq!(one["all"], false);
        assert_eq!(one["force"], true);
        assert_eq!(field(&one, "requested_by_id"), "U1");

        let all = run_request_body(None, "U1");
        assert!(all["key"].is_null());
        assert_eq!(all["all"], true);
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

    #[test]
    fn build_urlwatch_parsing() {
        // Without a selector: whole-page text diff.
        let u = build_urlwatch("Masters Results | https://usamasters.net/results").unwrap();
        assert_eq!(field(&u, "name"), "Masters Results");
        assert_eq!(field(&u, "url"), "https://usamasters.net/results");
        assert_eq!(u["filter"], json!(["html2text"]));

        // With a CSS selector: narrow, then html2text.
        let scoped = build_urlwatch("Records | https://usamasters.net/x | .kv-ee-content").unwrap();
        assert_eq!(scoped["filter"], json!([{ "css": ".kv-ee-content" }, "html2text"]));

        assert!(build_urlwatch("name only").is_err());
        assert!(build_urlwatch("name | ftp://nope").is_err());
    }
}
