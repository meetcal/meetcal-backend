//! Slack slash-command handler for managing scraper lists.
//!
//! One route backs all commands across all channels; it verifies the Slack
//! signature, routes the request to the list bound to the originating channel
//! (meet watches or entry targets), and dispatches `list` / `add` / `delete`.
//! The action is taken from the command name (e.g. `/meet-add`, `/entries-add`)
//! or the first word of the command text. Replies are ephemeral.
//!
//! The `/meets-add-pdf` / `/meets-add-map` / `/meets-remove-*` venue-map
//! commands are the one exception to the file handshake: they `UPDATE` the two
//! `venue_map_*` columns of `meets` directly, which is the only Postgres write
//! the API's `meetcal_api` role is granted (see the
//! `meetcal_api_venue_map_update` migration).

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::store::{JsonListStore, is_http_url, require_http_url, validate_slug};
use super::{ListKind, now_unix_secs, signature, write_json_request};
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

const USAMW_RESULTS_USAGE: &str = "*USAMW results*\n\
    • `/usamw-results <meet name> | <YYYY-MM-DD> | <pdf url> [<pdf url> ...]`\n\
    • Add `| adaptive` to mark the imported results as adaptive.";

const GENERIC_USAGE: &str = "Use a meet or entries command, e.g. `/meet-list`, \
    `/meet-add`, `/meet-run`, `/entries-list`, `/entries-add`, `/usamw-results`.";

const VENUE_MAP_USAGE: &str = "*Venue map links*\n\
    • `/meets-add-pdf \"MEET NAME\" <url>` — set the venue map PDF link\n\
    • `/meets-add-map \"MEET NAME\" <url>` — set the Apple Maps link\n\
    • `/meets-remove-pdf \"MEET NAME\"` — clear the venue map PDF link\n\
    • `/meets-remove-map \"MEET NAME\"` — clear the Apple Maps link\n\
    Meet names contain spaces, so quote them. The name must match the `name` \
    in the meets table exactly: copy it from the app's meet list or from \
    `GET /meets` / `GET /meets/completed` on the API.";

/// Defaults stamped on a watch created from Slack. They must stay in step with
/// the `MeetWatch` dataclass defaults in
/// `scrapers/usaw/meet_automation/config.py` (`DEFAULT_START_MEMBER_ID`,
/// `DEFAULT_SCHEDULE_START_ID`, `DEFAULT_MEET_YEAR`) -- the pipeline reads the
/// same `watches.json` this writes.
const DEFAULT_START_MEMBER_ID: i64 = 3100;
const DEFAULT_SCHEDULE_START_ID: i64 = 1;
const DEFAULT_MEET_YEAR: i64 = 2026;

/// Ceiling on PDF links accepted by one `/usamw-results` command. A results
/// import is a handful of session PDFs; the cap keeps a pasted wall of links
/// from queueing an unbounded download list for the scraper worker.
const MAX_USAMW_PDF_URLS: usize = 32;

/// Longest request key kept from a Slack `trigger_id` when naming a queued
/// request file. Real ids are ~40 chars (`13345224609.738474920.8088930838…`).
const MAX_REQUEST_KEY_LEN: usize = 64;
/// Hex digits of the body hash used as the request key when Slack sent no
/// `trigger_id` (16 hex = 64 bits: plenty to tell two commands apart).
const REQUEST_KEY_HASH_LEN: usize = 16;

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
    /// Unique per slash-command invocation. A replay of a signed request
    /// (inside the 5-minute signature window) carries the same one.
    #[serde(default)]
    trigger_id: String,
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

    if let Some(vm) = venue_map_command(&cmd.command) {
        return ephemeral(&venue_map_reply(&state.db, vm, &cmd.text).await);
    }

    if is_usamw_results_command(&cmd.command) {
        let key = request_key(&cmd.trigger_id, &body);
        return ephemeral(&usamw_results_reply(cfg, &cmd.text, &cmd.user_id, &key));
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
        (ListKind::Entries, Action::Run) => {
            ":information_source: `run` applies to meet watches. Try `/meet-run <key>`.".to_string()
        }
        (ListKind::Watches, action) => watches_reply(action, &args, &cfg.store_for(kind)),
        (ListKind::Entries, action) => entries_reply(action, &args, &cfg.store_for(kind)),
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
        "start_member_id": DEFAULT_START_MEMBER_ID,
        "schedule_start_id": DEFAULT_SCHEDULE_START_ID,
        "default_year": DEFAULT_MEET_YEAR,
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
            Ok(items)
                if !items
                    .iter()
                    .any(|i| store.key_of(i).eq_ignore_ascii_case(key)) =>
            {
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
    write_json_request(
        &cfg.run_requests_dir(),
        &run_request_filename(key),
        &run_request_body(key, user_id),
    )
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

// --- venue map links --------------------------------------------------------
/// The `/meets-*` venue-map commands write straight to the `meets` table in
/// Postgres (unlike the file-backed scraper lists above). Only the two
/// `venue_map_*` columns are writable by the API's database role.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VenueMapCommand {
    AddPdf,
    AddMap,
    RemovePdf,
    RemoveMap,
}

impl VenueMapCommand {
    /// The UPDATE statement for this command's column. Static strings keep
    /// sqlx's injection-safety bound satisfied (no dynamic SQL).
    fn update_sql(self) -> &'static str {
        match self {
            Self::AddPdf | Self::RemovePdf => {
                "UPDATE meets SET venue_map_pdf_url = $2 WHERE name = $1"
            }
            Self::AddMap | Self::RemoveMap => {
                "UPDATE meets SET venue_map_apple_url = $2 WHERE name = $1"
            }
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::AddPdf | Self::RemovePdf => "venue map PDF link",
            Self::AddMap | Self::RemoveMap => "Apple Maps link",
        }
    }

    fn is_add(self) -> bool {
        matches!(self, Self::AddPdf | Self::AddMap)
    }
}

fn venue_map_command(command: &str) -> Option<VenueMapCommand> {
    match command
        .trim_start_matches('/')
        .to_ascii_lowercase()
        .as_str()
    {
        "meets-add-pdf" => Some(VenueMapCommand::AddPdf),
        "meets-add-map" => Some(VenueMapCommand::AddMap),
        "meets-remove-pdf" => Some(VenueMapCommand::RemovePdf),
        "meets-remove-map" => Some(VenueMapCommand::RemoveMap),
        _ => None,
    }
}

/// What happened to a venue-map write, separated from the wording so the
/// wording can be unit-tested without a database.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VenueMapOutcome {
    /// No `meets` row carries that exact name.
    NoSuchMeet,
    /// The row exists but the UPDATE touched nothing: under row-level
    /// security that means the API's database role lacks the UPDATE grant or
    /// policy, not that the name was wrong.
    NotPermitted,
    Updated,
    /// The driver failed; the message is already in the server log.
    DatabaseError,
}

async fn venue_map_reply(db: &sqlx::PgPool, cmd: VenueMapCommand, text: &str) -> String {
    let (meet_name, value) = match parse_venue_map_args(cmd, text) {
        Ok(parsed) => parsed,
        Err(msg) => return format!(":warning: {msg}\n\n{VENUE_MAP_USAGE}"),
    };
    let outcome = apply_venue_map_update(db, cmd, &meet_name, value.as_deref()).await;
    venue_map_message(cmd, &meet_name, value.as_deref(), outcome)
}

/// Run the write. The existence check comes first so a `0 rows` UPDATE can be
/// told apart from a typo in the name: with FORCE ROW LEVEL SECURITY a missing
/// UPDATE policy silently filters every row instead of erroring.
async fn apply_venue_map_update(
    db: &sqlx::PgPool,
    cmd: VenueMapCommand,
    meet_name: &str,
    value: Option<&str>,
) -> VenueMapOutcome {
    let exists = sqlx::query_scalar::<_, i64>("SELECT 1::BIGINT FROM meets WHERE name = $1")
        .bind(meet_name)
        .fetch_optional(db)
        .await;
    match exists {
        Ok(None) => return VenueMapOutcome::NoSuchMeet,
        Ok(Some(_)) => {}
        // Same boundary rule as `AppError::Database`: the driver's message can
        // name tables, columns, and constraints, so it goes to the server log,
        // not into a Slack channel.
        Err(error) => {
            eprintln!("venue map lookup failed: {error}");
            return VenueMapOutcome::DatabaseError;
        }
    }

    // Overwrites silently by design; the meet name must match exactly.
    match sqlx::query(cmd.update_sql())
        .bind(meet_name)
        .bind(value)
        .execute(db)
        .await
    {
        Ok(done) if done.rows_affected() == 0 => VenueMapOutcome::NotPermitted,
        Ok(_) => VenueMapOutcome::Updated,
        Err(error) => {
            eprintln!("venue map update failed: {error}");
            VenueMapOutcome::DatabaseError
        }
    }
}

fn venue_map_message(
    cmd: VenueMapCommand,
    meet_name: &str,
    value: Option<&str>,
    outcome: VenueMapOutcome,
) -> String {
    match outcome {
        VenueMapOutcome::NoSuchMeet => format!(
            ":mag: No meet named `{meet_name}`. Names must match the meets table \
             exactly — copy the `name` from the app's meet list or `GET /meets`."
        ),
        VenueMapOutcome::NotPermitted => format!(
            ":no_entry: `{meet_name}` exists but the API's database role was not \
             allowed to update it. Check the `meetcal_api` UPDATE grant and policy \
             on `meets` (migration `meetcal_api_venue_map_update`)."
        ),
        VenueMapOutcome::Updated => match value {
            Some(url) => format!(
                ":white_check_mark: Set the {} for `{meet_name}` to {url}.",
                cmd.label()
            ),
            None => format!(
                ":white_check_mark: Removed the {} for `{meet_name}`.",
                cmd.label()
            ),
        },
        VenueMapOutcome::DatabaseError => {
            ":warning: Database error updating the meet; check the server log.".to_string()
        }
    }
}

/// Parse `"MEET NAME" [url]`: a quoted meet name (names contain spaces), then a
/// URL for the add commands. Accepts straight or Slack "smart" quotes.
fn parse_venue_map_args(
    cmd: VenueMapCommand,
    text: &str,
) -> Result<(String, Option<String>), String> {
    let text = text.trim();
    let mut chars = text.chars();
    if !matches!(chars.next(), Some('"' | '\u{201C}' | '\u{201D}')) {
        return Err("the meet name must be wrapped in quotes, e.g. \
             `\"2026 Ohio WSO Championships\"`"
            .to_string());
    }
    let after_open = chars.as_str();
    let Some(close) = after_open.find(['"', '\u{201C}', '\u{201D}']) else {
        return Err("missing closing quote on the meet name".to_string());
    };
    let meet_name = after_open[..close].trim().to_string();
    if meet_name.is_empty() {
        return Err("missing meet name".to_string());
    }
    let rest = after_open[close..]
        .trim_start_matches(['"', '\u{201C}', '\u{201D}'])
        .trim();

    if cmd.is_add() {
        if rest.is_empty() {
            return Err("missing <url> after the meet name".to_string());
        }
        // Slack wraps pasted links in <...>; unwrap before storing.
        let url = rest
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(['<', '>'])
            .to_string();
        if url.is_empty() {
            return Err("missing <url> after the meet name".to_string());
        }
        // The link is served to every app user, so only http(s) schemes are
        // stored: no `javascript:` / `file:` links via a Slack command.
        require_http_url("url", Some(&url))?;
        Ok((meet_name, Some(url)))
    } else {
        Ok((meet_name, None))
    }
}

// --- USAMW results --------------------------------------------------------
fn is_usamw_results_command(command: &str) -> bool {
    matches!(
        command
            .trim_start_matches('/')
            .to_ascii_lowercase()
            .as_str(),
        "usamw-results" | "usamw-results-scrape" | "usamwresultsscraper"
    )
}

fn usamw_results_reply(
    cfg: &super::SlackConfig,
    text: &str,
    user_id: &str,
    request_key: &str,
) -> String {
    match build_usamw_results_request(text, user_id) {
        Ok(body) => match queue_usamw_results(cfg, &body, request_key) {
            Ok(()) => {
                let count = body["pdf_urls"].as_array().map(Vec::len).unwrap_or(0);
                format!(
                    ":hourglass_flowing_sand: Queued USAMW results import for `{}` \
                     ({} PDF link{}). The worker will scrape and write to Postgres.",
                    field(&body, "meet"),
                    count,
                    if count == 1 { "" } else { "s" }
                )
            }
            Err(msg) => format!(":warning: Could not queue USAMW results import: {msg}"),
        },
        Err(msg) => format!(":warning: {msg}\n\n{USAMW_RESULTS_USAGE}"),
    }
}

fn build_usamw_results_request(text: &str, user_id: &str) -> Result<Value, String> {
    let fields: Vec<String> = text
        .split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if fields.len() < 3 {
        return Err("missing meet name, date, or PDF URL".to_string());
    }

    let meet = fields[0].clone();
    let date = fields[1].clone();
    validate_date(&date)?;

    let adaptive = fields[2..]
        .iter()
        .any(|field| field.eq_ignore_ascii_case("adaptive") || field.eq_ignore_ascii_case("true"));

    let mut urls = Vec::new();
    for field in &fields[2..] {
        for token in field.split_whitespace() {
            let token = token.trim_matches(|c: char| matches!(c, ',' | '<' | '>' | '"'));
            if token.eq_ignore_ascii_case("adaptive") || token.eq_ignore_ascii_case("true") {
                continue;
            }
            if is_http_url(token) {
                require_http_url("PDF URL", Some(token))?;
                if urls.len() >= MAX_USAMW_PDF_URLS {
                    return Err(format!(
                        "too many PDF URLs (limit {MAX_USAMW_PDF_URLS}); split the import"
                    ));
                }
                urls.push(token.to_string());
            }
        }
    }

    if urls.is_empty() {
        return Err("missing PDF URL".to_string());
    }

    Ok(json!({
        "meet": meet,
        "date": date,
        "adaptive": adaptive,
        "pdf_urls": urls,
        "requested_by_id": user_id,
        "requested_at_unix": now_unix_secs(),
    }))
}

fn validate_date(date: &str) -> Result<(), String> {
    let bytes = date.as_bytes();
    if bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        Ok(())
    } else {
        Err("date must be YYYY-MM-DD".to_string())
    }
}

/// Drop the request file. Its name is derived from the request (not from the
/// clock at handling time), so a replayed Slack request inside the signature
/// window overwrites its own file instead of queueing a second import.
fn queue_usamw_results(
    cfg: &super::SlackConfig,
    body: &Value,
    request_key: &str,
) -> Result<(), String> {
    write_json_request(
        &cfg.usamw_results_requests_dir(),
        &usamw_request_filename(&field(body, "meet"), request_key),
        body,
    )
}

fn usamw_request_filename(meet: &str, request_key: &str) -> String {
    format!("{}-{request_key}.json", slugify(meet))
}

/// A filename-safe key that identifies one Slack request: the `trigger_id`
/// Slack assigns per invocation, or, when absent, a hash of the raw signed
/// body. Either is identical on a replay and different for a fresh command.
fn request_key(trigger_id: &str, body: &[u8]) -> String {
    let from_trigger: String = trigger_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(MAX_REQUEST_KEY_LEN)
        .collect();
    if !from_trigger.is_empty() {
        return from_trigger;
    }
    let digest = Sha256::digest(body);
    hex::encode(digest)[..REQUEST_KEY_HASH_LEN].to_string()
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in value.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

// --- entry targets --------------------------------------------------------
fn entries_reply(action: Action, args: &str, store: &JsonListStore) -> String {
    match action {
        Action::List => match store.items() {
            Ok(items) if items.is_empty() => "No meet entries are being scraped.".to_string(),
            Ok(items) => {
                let mut out = format!("*Scraping entries for {} target(s):*", items.len());
                for e in items {
                    out.push_str(&format!(
                        "\n• `{}` — {}",
                        field(&e, "label"),
                        field(&e, "url")
                    ));
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
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
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
        // Generic command falls back to the first word of the text.
        assert_eq!(
            route_kind("/scraper", "entries list"),
            Some(ListKind::Entries)
        );
        assert_eq!(route_kind("/scraper", "meet list"), Some(ListKind::Watches));
        assert_eq!(route_kind("/scraper", "huh"), None);
    }

    #[test]
    fn dispatch_by_command_name() {
        assert!(matches!(parse_action("/meet-list", ""), (Action::List, _)));
        assert!(matches!(
            parse_action("/entries-add", "a|b"),
            (Action::Add, _)
        ));
        assert!(matches!(
            parse_action("/meet-delete", "k"),
            (Action::Delete, _)
        ));
        assert!(matches!(
            parse_action("/meet-run", "2026-nats"),
            (Action::Run, _)
        ));
    }

    #[test]
    fn run_routes_to_watches() {
        // `/meet-run` carries "meet", so it targets the watches list.
        assert_eq!(
            route_kind("/meet-run", "2026-nats"),
            Some(ListKind::Watches)
        );
    }

    #[test]
    fn usamw_results_request_parsing() {
        let body = build_usamw_results_request(
            "2026 USA Masters Nationals | 2026-03-29 | https://e.com/a.pdf https://e.com/b.pdf | adaptive",
            "U1",
        )
        .unwrap();
        assert_eq!(field(&body, "meet"), "2026 USA Masters Nationals");
        assert_eq!(field(&body, "date"), "2026-03-29");
        assert_eq!(body["adaptive"], true);
        assert_eq!(body["pdf_urls"].as_array().unwrap().len(), 2);
        assert_eq!(field(&body, "requested_by_id"), "U1");

        assert!(build_usamw_results_request("Meet | nope | https://e.com/a.pdf", "U1").is_err());
        assert!(build_usamw_results_request("Meet | 2026-03-29", "U1").is_err());
    }

    #[test]
    fn usamw_results_pdf_urls_are_bounded() {
        let links = |count: usize| {
            let urls: Vec<String> = (0..count)
                .map(|i| format!("https://e.com/{i}.pdf"))
                .collect();
            format!("Meet | 2026-03-29 | {}", urls.join(" "))
        };

        // Zero PDF links is already a usage error; one and many are accepted.
        assert!(build_usamw_results_request("Meet | 2026-03-29 | adaptive", "U1").is_err());
        assert_eq!(
            build_usamw_results_request(&links(1), "U1").unwrap()["pdf_urls"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        // Max: exactly the cap is fine, one past it is rejected rather than
        // queueing an unbounded download list.
        assert_eq!(
            build_usamw_results_request(&links(MAX_USAMW_PDF_URLS), "U1").unwrap()["pdf_urls"]
                .as_array()
                .unwrap()
                .len(),
            MAX_USAMW_PDF_URLS
        );
        let error = build_usamw_results_request(&links(MAX_USAMW_PDF_URLS + 1), "U1").unwrap_err();
        assert!(error.contains("too many PDF URLs"), "{error}");
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
        assert_eq!(
            parse_run_target("2026-nats").unwrap(),
            Some("2026-nats".to_string())
        );
        // Only the first token is taken as the key.
        assert_eq!(
            parse_run_target("2026-nats now").unwrap(),
            Some("2026-nats".to_string())
        );
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

        let full =
            build_watch("k | n | https://e.com/p | https://e.com/s | https://e.com/c").unwrap();
        assert_eq!(field(&full, "start_list_url"), "https://e.com/s");

        assert!(build_watch("only-key").is_err());
        assert!(build_watch("bad key | n | https://e.com").is_err());
    }

    #[test]
    fn request_key_is_stable_per_request_and_filename_safe() {
        // Same trigger id (a replay) ⇒ same key ⇒ same file.
        assert_eq!(
            request_key("13345224609.738474920.8088930838d88f008e0", b"a"),
            "13345224609.738474920.8088930838d88f008e0"
        );
        // Hostile characters never reach the filename.
        assert_eq!(request_key("../x/y", b"a"), "..xy");
        assert_eq!(request_key("a b", b"a"), "ab");
        // Bounded even when Slack sends something absurd.
        assert_eq!(
            request_key(&"z".repeat(MAX_REQUEST_KEY_LEN * 2), b"a").len(),
            MAX_REQUEST_KEY_LEN
        );
        // No trigger id: the body hash stands in, still stable and bounded.
        let hashed = request_key("", b"command=%2Fusamw-results&text=x");
        assert_eq!(hashed.len(), REQUEST_KEY_HASH_LEN);
        assert!(hashed.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hashed, request_key("", b"command=%2Fusamw-results&text=x"));
        assert_ne!(hashed, request_key("", b"command=%2Fusamw-results&text=y"));
        assert_eq!(
            usamw_request_filename("2026 USA Masters Nationals", &hashed),
            format!("2026-usa-masters-nationals-{hashed}.json")
        );
    }

    #[test]
    fn venue_map_messages_distinguish_missing_meet_from_denied_update() {
        let no_such = venue_map_message(
            VenueMapCommand::AddPdf,
            "No Such Meet",
            Some("https://e.com/x.pdf"),
            VenueMapOutcome::NoSuchMeet,
        );
        assert!(no_such.contains("No meet named"), "{no_such}");
        assert!(!no_such.contains("/meets-list"), "{no_such}");

        let denied = venue_map_message(
            VenueMapCommand::AddPdf,
            "2026 Nationals",
            Some("https://e.com/x.pdf"),
            VenueMapOutcome::NotPermitted,
        );
        assert!(denied.contains("not"), "{denied}");
        assert!(denied.contains("meetcal_api"), "{denied}");
        assert!(!denied.contains("No meet named"), "{denied}");

        assert!(
            venue_map_message(
                VenueMapCommand::RemoveMap,
                "2026 Nationals",
                None,
                VenueMapOutcome::Updated
            )
            .contains("Removed the Apple Maps link")
        );
        assert!(!VENUE_MAP_USAGE.contains("/meets-list"));
    }

    #[test]
    fn venue_map_rejects_non_http_urls() {
        for bad in [
            "\"2026 Nationals\" javascript:alert(1)",
            "\"2026 Nationals\" file:///etc/passwd",
            "\"2026 Nationals\" <ftp://e.com/map.pdf>",
            "\"2026 Nationals\" e.com/map.pdf",
        ] {
            let error = parse_venue_map_args(VenueMapCommand::AddPdf, bad).unwrap_err();
            assert!(error.contains("http(s)"), "{bad}: {error}");
        }
        assert!(
            parse_venue_map_args(VenueMapCommand::AddMap, "\"2026 Nationals\" http://e.com/m")
                .is_ok()
        );
    }

    #[test]
    fn venue_map_command_routing() {
        assert_eq!(
            venue_map_command("/meets-add-pdf"),
            Some(VenueMapCommand::AddPdf)
        );
        assert_eq!(
            venue_map_command("/meets-add-map"),
            Some(VenueMapCommand::AddMap)
        );
        assert_eq!(
            venue_map_command("/meets-remove-pdf"),
            Some(VenueMapCommand::RemovePdf)
        );
        assert_eq!(
            venue_map_command("/meets-remove-map"),
            Some(VenueMapCommand::RemoveMap)
        );
        // Existing commands must not be swallowed.
        assert_eq!(venue_map_command("/meet-add"), None);
        assert_eq!(venue_map_command("/meets-list"), None);
    }

    #[test]
    fn venue_map_args_parsing() {
        let (name, url) = parse_venue_map_args(
            VenueMapCommand::AddPdf,
            "\"2026 Ohio WSO Championships\" https://e.com/map.pdf",
        )
        .unwrap();
        assert_eq!(name, "2026 Ohio WSO Championships");
        assert_eq!(url.as_deref(), Some("https://e.com/map.pdf"));

        // Slack smart quotes and <...>-wrapped links.
        let (name, url) = parse_venue_map_args(
            VenueMapCommand::AddMap,
            "\u{201C}2026 Nationals\u{201D} <https://maps.apple.com/?q=1>",
        )
        .unwrap();
        assert_eq!(name, "2026 Nationals");
        assert_eq!(url.as_deref(), Some("https://maps.apple.com/?q=1"));

        // Remove takes just the quoted name.
        let (name, url) =
            parse_venue_map_args(VenueMapCommand::RemovePdf, "\"2026 Nationals\"").unwrap();
        assert_eq!(name, "2026 Nationals");
        assert_eq!(url, None);

        assert!(parse_venue_map_args(VenueMapCommand::AddPdf, "no quotes here").is_err());
        assert!(parse_venue_map_args(VenueMapCommand::AddPdf, "\"unclosed name").is_err());
        assert!(parse_venue_map_args(VenueMapCommand::AddPdf, "\"2026 Nationals\"").is_err());
        assert!(parse_venue_map_args(VenueMapCommand::AddPdf, "\"\" https://e.com").is_err());
    }

    #[test]
    fn build_entry_parsing() {
        let e = build_entry(
            "Masters Nats | https://usaweightlifting.sport80.com/public/events/1/entries/2",
        )
        .unwrap();
        assert_eq!(field(&e, "label"), "Masters Nats");
        assert!(build_entry("no url").is_err());
        assert!(build_entry("label | ftp://nope").is_err());
    }
}
