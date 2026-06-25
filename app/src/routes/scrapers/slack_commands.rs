//! Slack slash-command handler for managing meet-automation watches.
//!
//! One route backs all three commands; point `/list`, `/add`, `/delete` (or
//! namespaced variants like `/meet-list`) at it and they dispatch on the
//! command name, falling back to the first word of the command text. Every
//! request is signature-verified and optionally restricted to one channel and
//! an allowlist of users.
//!
//! Usage shown to users:
//!   /…-list
//!   /…-add  <key> | <meet name> | <page url> [| <start-list url> | <schedule url>]
//!   /…-delete <key>

use axum::{
    Json,
    body::Bytes,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use super::signature;
use super::watches::NewWatch;
use crate::AppState;

const USAGE: &str = "*Meet watches*\n\
    • `list` — show watched meet pages\n\
    • `add <key> | <meet name> | <page url> [| <start-list url> | <schedule url>]`\n\
    • `delete <key>`";

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
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let cfg = &state.slack;
    if !cfg.enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Slack meet-automation commands are not configured",
        )
            .into_response();
    }

    // 1. Verify the request really came from Slack (signature over raw body).
    let timestamp = header(&headers, "x-slack-request-timestamp");
    let provided_sig = header(&headers, "x-slack-signature");
    let (Some(timestamp), Some(provided_sig)) = (timestamp, provided_sig) else {
        return (StatusCode::UNAUTHORIZED, "missing Slack signature headers").into_response();
    };
    if !signature::verify(&cfg.signing_secret, &timestamp, &body, &provided_sig) {
        return (StatusCode::UNAUTHORIZED, "bad Slack signature").into_response();
    }

    // 2. Parse the urlencoded payload.
    let Ok(cmd) = serde_urlencoded::from_bytes::<SlackCommand>(&body) else {
        return ephemeral("could not parse slash command payload");
    };

    // 3. Channel + user allowlist.
    if !cfg.channel_id.is_empty() && cmd.channel_id != cfg.channel_id {
        return ephemeral("These commands aren't enabled in this channel.");
    }
    if !cfg.allowed_users.is_empty() && !cfg.allowed_users.iter().any(|u| u == &cmd.user_id) {
        return ephemeral("You're not authorized to manage meet watches.");
    }

    // 4. Dispatch.
    let (action, args) = parse_action(&cmd.command, &cmd.text);
    let store = cfg.store();
    let text = match action {
        Action::List => match store.list() {
            Ok(watches) if watches.is_empty() => "No meet pages are being watched.".to_string(),
            Ok(watches) => {
                let mut out = format!("*Watching {} meet page(s):*", watches.len());
                for w in watches {
                    out.push_str(&format!("\n• `{}` — {}\n   {}", w.key, w.meet_name, w.page_url));
                }
                out
            }
            Err(msg) => format!(":warning: {msg}"),
        },
        Action::Add => match parse_add(&args) {
            Ok(spec) => {
                let key = spec.key.clone();
                match store.add(spec) {
                    Ok(()) => format!(":white_check_mark: Added watch `{key}`."),
                    Err(msg) => format!(":warning: {msg}"),
                }
            }
            Err(msg) => format!(":warning: {msg}\n\n{USAGE}"),
        },
        Action::Delete => {
            let key = args.split_whitespace().next().unwrap_or("").to_string();
            if key.is_empty() {
                format!(":warning: usage: `delete <key>`\n\n{USAGE}")
            } else {
                match store.delete(&key) {
                    Ok(true) => format!(":white_check_mark: Deleted watch `{key}`."),
                    Ok(false) => format!(":mag: No watch with key `{key}`."),
                    Err(msg) => format!(":warning: {msg}"),
                }
            }
        }
        Action::Help => USAGE.to_string(),
    };

    ephemeral(&text)
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_string)
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

fn parse_add(args: &str) -> Result<NewWatch, String> {
    let fields: Vec<String> = args.split('|').map(|s| s.trim().to_string()).collect();
    let get = |i: usize| fields.get(i).filter(|s| !s.is_empty()).cloned();

    let key = get(0).ok_or("missing <key>")?;
    let meet_name = get(1).ok_or("missing <meet name>")?;
    let page_url = get(2).ok_or("missing <page url>")?;

    Ok(NewWatch {
        key,
        meet_name,
        page_url,
        start_list_url: get(3),
        schedule_url: get(4),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_by_command_name() {
        assert!(matches!(parse_action("/meet-list", ""), (Action::List, _)));
        assert!(matches!(parse_action("/add", "a|b|c"), (Action::Add, _)));
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
    fn add_parsing() {
        let spec = parse_add("2026-nats | 2026 Nationals | https://e.com/p").unwrap();
        assert_eq!(spec.key, "2026-nats");
        assert_eq!(spec.meet_name, "2026 Nationals");
        assert_eq!(spec.page_url, "https://e.com/p");
        assert!(spec.start_list_url.is_none());

        let full =
            parse_add("k | n | https://e.com/p | https://e.com/s | https://e.com/sch").unwrap();
        assert_eq!(full.start_list_url.as_deref(), Some("https://e.com/s"));
        assert_eq!(full.schedule_url.as_deref(), Some("https://e.com/sch"));

        assert!(parse_add("only-key").is_err());
    }
}
