//! Slack interactive-component handler for staged meet uploads.
//!
//! The meet-automation pipeline posts each staged run with *Approve & publish*
//! and *Reject* buttons. When clicked, Slack POSTs here. We verify the
//! signature, then record the decision as a file under
//! `<state_dir>/decisions/<run_id>.json`. The Python `approve` cron picks that
//! up and performs the actual Postgres write, posting a confirmation.
//! confirmations.
//!
//! Keeping the DB write in Python means this read-only API never gains database
//! credentials; the button just drops a decision token on the shared filesystem.
//! We also update the original Slack message via `response_url` for instant
//! feedback (best-effort).

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::now_unix_secs;
use super::signature;
use crate::AppState;

const ACTION_APPROVE: &str = "meet_approve";
const ACTION_REJECT: &str = "meet_reject";

#[derive(Deserialize, Default)]
struct InteractionForm {
    #[serde(default)]
    payload: String,
}

pub async fn slack_interactions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let cfg = &state.slack;
    if !cfg.enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Slack interactions are not configured",
        )
            .into_response();
    }
    if let Some(resp) = signature::require_valid(&cfg.signing_secret, &headers, &body) {
        return resp;
    }

    let Ok(form) = serde_urlencoded::from_bytes::<InteractionForm>(&body) else {
        return (StatusCode::BAD_REQUEST, "missing interaction payload").into_response();
    };
    let Ok(payload) = serde_json::from_str::<Value>(&form.payload) else {
        return (StatusCode::BAD_REQUEST, "invalid interaction payload").into_response();
    };

    if payload.get("type").and_then(Value::as_str) != Some("block_actions") {
        return StatusCode::OK.into_response(); // ack anything we don't handle
    }

    let action = payload
        .get("actions")
        .and_then(Value::as_array)
        .and_then(|a| a.first());
    let action_id = action
        .and_then(|a| a.get("action_id"))
        .and_then(Value::as_str);
    let run_id = action
        .and_then(|a| a.get("value"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let user_id = payload
        .pointer("/user/id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let user_name = payload
        .pointer("/user/username")
        .or_else(|| payload.pointer("/user/name"))
        .and_then(Value::as_str)
        .unwrap_or("someone");
    let response_url = payload
        .get("response_url")
        .and_then(Value::as_str)
        .map(str::to_string);

    // Optional user allowlist (same as slash commands).
    if !cfg.user_allowed(user_id) {
        update_message(
            response_url,
            ":no_entry: You're not authorized to approve uploads.",
        )
        .await;
        return StatusCode::OK.into_response();
    }

    let decision = match action_id {
        Some(ACTION_APPROVE) => "approved",
        Some(ACTION_REJECT) => "rejected",
        _ => return StatusCode::OK.into_response(),
    };

    let feedback = match write_decision(&state, run_id, decision, user_id, user_name) {
        Ok(()) => match decision {
            "approved" => {
                format!(":white_check_mark: Approved by <@{user_id}> — publishing shortly…")
            }
            _ => format!(":wastebasket: Rejected by <@{user_id}> — discarded."),
        },
        Err(msg) => format!(":warning: Could not record decision: {msg}"),
    };
    update_message(response_url, &feedback).await;

    StatusCode::OK.into_response()
}

/// Persist the decision for the Python `approve` cron to consume.
fn write_decision(
    state: &AppState,
    run_id: &str,
    decision: &str,
    user_id: &str,
    user_name: &str,
) -> Result<(), String> {
    if !is_safe_run_id(run_id) {
        return Err("invalid run id".to_string());
    }
    let dir = state.slack.decisions_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create dir: {e}"))?;
    let body = json!({
        "decision": decision,
        "user_id": user_id,
        "user_name": user_name,
        "decided_at_unix": now_unix_secs(),
    });
    let path = dir.join(format!("{run_id}.json"));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&body).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("write: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

/// Replace the original Slack message (instant feedback). Best-effort.
async fn update_message(response_url: Option<String>, text: &str) {
    let Some(url) = response_url else { return };
    let client = reqwest::Client::new();
    let _ = client
        .post(url)
        .json(&json!({ "replace_original": true, "text": text }))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
}

fn is_safe_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id.len() <= 200
        // Reject `.`/`..` outright: with the `.json` suffix they can't escape the
        // decisions dir, but they're never valid run ids and shouldn't write a file.
        && run_id != "."
        && run_id != ".."
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_id_safety() {
        assert!(is_safe_run_id("2026-nationals-20260625-195426"));
        assert!(!is_safe_run_id(""));
        assert!(!is_safe_run_id("."));
        assert!(!is_safe_run_id(".."));
        assert!(!is_safe_run_id("../../etc/passwd"));
        assert!(!is_safe_run_id("a/b"));
    }
}
