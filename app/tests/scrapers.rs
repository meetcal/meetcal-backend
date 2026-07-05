//! Integration tests for the Slack scraper-control endpoints. Exercises the
//! real HTTP routes with genuine Slack signatures against a spawned server.
//!
//! All assertions live in one test because they share process-global env
//! (signing secret + file paths), avoiding cross-test races.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

mod support;

const SECRET: &str = "itest-signing-secret";

fn sign(ts: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("v0:{ts}:{body}").as_bytes());
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
}

fn now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string()
}

async fn post(
    client: &reqwest::Client,
    url: &str,
    body: &str,
    ts: &str,
    sig: &str,
) -> reqwest::Response {
    client
        .post(url)
        .header("x-slack-request-timestamp", ts)
        .header("x-slack-signature", sig)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body.to_string())
        .send()
        .await
        .unwrap()
}

async fn post_signed(client: &reqwest::Client, url: &str, body: &str) -> reqwest::Response {
    let ts = now();
    let sig = sign(&ts, body);
    post(client, url, body, &ts, &sig).await
}

#[tokio::test]
async fn slack_scraper_control_endpoints() {
    // Unique temp workspace for this run's list + decision files.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "meetcal-itest-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let watches_path = tmp.join("watches.json");
    let entries_path = tmp.join("entries_targets.json");

    // SlackConfig::from_env() is read when the server starts, so set env first.
    // SAFETY: single-threaded test setup before the server spawns.
    unsafe {
        std::env::set_var("SLACK_SIGNING_SECRET", SECRET);
        std::env::set_var("MEET_AUTOMATION_WATCHES_PATH", &watches_path);
        std::env::set_var("ENTRIES_TARGETS_PATH", &entries_path);
        std::env::set_var("MEET_AUTOMATION_STATE_DIR", &tmp);
        std::env::remove_var("SLACK_MEET_AUTOMATION_CHANNEL");
        std::env::remove_var("SLACK_ENTRIES_CHANNEL");
        std::env::remove_var("MEET_AUTOMATION_SLACK_ALLOWED_USERS");
    }

    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let cmd_url = format!("{}/scrapers/slack/commands", app.address);
    let int_url = format!("{}/scrapers/slack/interactions", app.address);

    // --- bad signature is rejected -------------------------------------
    let ts = now();
    let resp = post(
        &client,
        &cmd_url,
        "command=%2Fmeet-list",
        &ts,
        "v0=deadbeef",
    )
    .await;
    assert_eq!(resp.status(), 401, "bad signature must be rejected");

    // --- watches list starts empty -------------------------------------
    let resp = post_signed(&client, &cmd_url, "command=%2Fmeet-list&text=").await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["text"].as_str().unwrap().contains("No meet pages"));

    // --- add a watch (routed by command name, any channel) -------------
    let add = "command=%2Fmeet-add&text=2026-itest+%7C+2026+Itest+Meet+%7C+https%3A%2F%2Fe.com%2Fp";
    let resp = post_signed(&client, &cmd_url, add).await;
    assert_eq!(resp.status(), 200);
    assert!(
        resp.json::<serde_json::Value>().await.unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("Added watch")
    );

    let watches: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&watches_path).unwrap()).unwrap();
    assert_eq!(watches[0]["key"], "2026-itest");
    assert_eq!(watches[0]["start_member_id"], 3100);

    // --- add an entry target in the SAME workspace (different command) --
    let add_e =
        "command=%2Fentries-add&text=Masters+%7C+https%3A%2F%2Fe.com%2Fevents%2F1%2Fentries%2F2";
    let resp = post_signed(&client, &cmd_url, add_e).await;
    assert_eq!(resp.status(), 200);
    let entries: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&entries_path).unwrap()).unwrap();
    assert_eq!(entries[0]["label"], "Masters");

    // --- delete the watch ----------------------------------------------
    let resp = post_signed(&client, &cmd_url, "command=%2Fmeet-delete&text=2026-itest").await;
    assert_eq!(resp.status(), 200);
    let watches: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&watches_path).unwrap()).unwrap();
    assert!(watches.as_array().unwrap().is_empty());

    // --- queue a USAMW results import ----------------------------------
    let usamw_body = "command=%2Fusamw-results&text=2026+USA+Masters+Nationals+%7C+2026-03-29+%7C+https%3A%2F%2Fe.com%2Fa.pdf+https%3A%2F%2Fe.com%2Fb.pdf+%7C+adaptive";
    let resp = post_signed(&client, &cmd_url, usamw_body).await;
    assert_eq!(resp.status(), 200);
    assert!(
        resp.json::<serde_json::Value>().await.unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("Queued USAMW results import")
    );

    let request_dir = tmp.join("usamw_results_requests");
    let request_file = std::fs::read_dir(&request_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let request: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(request_file).unwrap()).unwrap();
    assert_eq!(request["meet"], "2026 USA Masters Nationals");
    assert_eq!(request["date"], "2026-03-29");
    assert_eq!(request["adaptive"], true);
    assert_eq!(request["pdf_urls"].as_array().unwrap().len(), 2);

    // --- a button click records a decision file ------------------------
    let payload = serde_json::json!({
        "type": "block_actions",
        "user": {"id": "U1", "username": "tester"},
        "channel": {"id": "C1"},
        "actions": [{"action_id": "meet_approve", "value": "itest-run-1"}]
    })
    .to_string();
    let body = serde_urlencoded::to_string([("payload", payload)]).unwrap();
    let resp = post_signed(&client, &int_url, &body).await;
    assert_eq!(resp.status(), 200);

    let decision: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.join("decisions/itest-run-1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(decision["decision"], "approved");
    assert_eq!(decision["user_id"], "U1");

    let _ = std::fs::remove_dir_all(&tmp);
}
