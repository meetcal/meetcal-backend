//! Integration tests for the Slack scraper-control endpoints. Exercises the
//! real HTTP routes with genuine Slack signatures against a spawned server.
//!
//! All assertions live in one test because they share process-global env
//! (signing secret + file paths), avoiding cross-test races. The test spawns
//! two servers in sequence: one with no allowlists, then one with a channel +
//! user allowlist, since `SlackConfig` is read from the env at spawn time.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::Acquire;
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

    // --- missing signature headers are rejected on both endpoints --------
    for url in [&cmd_url, &int_url] {
        let resp = client
            .post(url)
            .header("content-type", "application/x-www-form-urlencoded")
            .body("command=%2Fmeet-list")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401, "missing headers must be rejected");
    }

    // --- a correctly signed but stale request is a replay ----------------
    let stale = (now().parse::<i64>().unwrap() - 6 * 60).to_string();
    let body = "command=%2Fmeet-list";
    let resp = post(&client, &cmd_url, body, &stale, &sign(&stale, body)).await;
    assert_eq!(resp.status(), 401, "stale timestamp must be rejected");

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
    let request_files = || -> Vec<std::path::PathBuf> {
        let mut files: Vec<_> = std::fs::read_dir(&request_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        files
    };
    let request_file = request_files().into_iter().next().unwrap();
    let request: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&request_file).unwrap()).unwrap();
    assert_eq!(request["meet"], "2026 USA Masters Nationals");
    assert_eq!(request["date"], "2026-03-29");
    assert_eq!(request["adaptive"], true);
    assert_eq!(request["pdf_urls"].as_array().unwrap().len(), 2);

    // --- a replay inside the signature window does not queue twice --------
    // Same body, timestamp and signature, exactly as a captured request
    // would be resent: the file is overwritten, not duplicated.
    let replay_body = format!("{usamw_body}&trigger_id=111.222.abc");
    let ts = now();
    let sig = sign(&ts, &replay_body);
    for _ in 0..2 {
        let resp = post(&client, &cmd_url, &replay_body, &ts, &sig).await;
        assert_eq!(resp.status(), 200);
    }
    assert_eq!(request_files().len(), 2, "one file per distinct request");
    assert!(
        request_files().iter().any(|p| p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-111.222.abc.json")),
        "the file is keyed on Slack's trigger_id"
    );

    // --- venue map links (writes to the meets table) --------------------
    let meet = "2026 Ohio WSO Championships";
    let details_url = format!(
        "{}/meets/details?meet=2026%20Ohio%20WSO%20Championships",
        app.address
    );

    // Set the PDF link and confirm it shows up in the meets API.
    let body = serde_urlencoded::to_string([
        ("command", "/meets-add-pdf"),
        ("text", &format!("\"{meet}\" https://e.com/venue-map.pdf")),
    ])
    .unwrap();
    let resp = post_signed(&client, &cmd_url, &body).await;
    assert_eq!(resp.status(), 200);
    assert!(
        resp.json::<serde_json::Value>().await.unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("Set the venue map PDF link")
    );
    let details: serde_json::Value = reqwest::get(&details_url)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(details["venue_map_pdf_url"], "https://e.com/venue-map.pdf");

    // Set the Apple Maps link (smart quotes + <...>-wrapped URL, as Slack sends).
    let body = serde_urlencoded::to_string([
        ("command", "/meets-add-map"),
        (
            "text",
            &format!("\u{201C}{meet}\u{201D} <https://maps.apple.com/?q=venue>"),
        ),
    ])
    .unwrap();
    let resp = post_signed(&client, &cmd_url, &body).await;
    assert_eq!(resp.status(), 200);
    let details: serde_json::Value = reqwest::get(&details_url)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        details["venue_map_apple_url"],
        "https://maps.apple.com/?q=venue"
    );

    // A name that doesn't match exactly is an error.
    let body = serde_urlencoded::to_string([
        ("command", "/meets-add-pdf"),
        ("text", "\"No Such Meet\" https://e.com/x.pdf"),
    ])
    .unwrap();
    let resp = post_signed(&client, &cmd_url, &body).await;
    assert!(
        resp.json::<serde_json::Value>().await.unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("No meet named")
    );

    // A non-http(s) link is refused before any write.
    let body = serde_urlencoded::to_string([
        ("command", "/meets-add-pdf"),
        ("text", &format!("\"{meet}\" javascript:alert(1)")),
    ])
    .unwrap();
    let resp = post_signed(&client, &cmd_url, &body).await;
    assert!(
        resp.json::<serde_json::Value>().await.unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("http(s)")
    );
    let details: serde_json::Value = reqwest::get(&details_url)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(details["venue_map_pdf_url"], "https://e.com/venue-map.pdf");

    // The same UPDATE the command runs must be allowed to the production
    // role (the test server connects as the superuser, which bypasses RLS).
    {
        let db = support::db_pool().await;
        let mut conn = db.acquire().await.unwrap();
        sqlx::query("SET ROLE meetcal_api")
            .execute(&mut *conn)
            .await
            .unwrap();
        let mut tx = conn.begin().await.unwrap();
        let updated = sqlx::query("UPDATE meets SET venue_map_pdf_url = $2 WHERE name = $1")
            .bind(meet)
            .bind("https://e.com/role.pdf")
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected();
        assert_eq!(
            updated, 1,
            "meetcal_api must be able to set venue map links"
        );
        // ...and only those two columns.
        let denied = sqlx::query("UPDATE meets SET status = 'x' WHERE name = $1")
            .bind(meet)
            .execute(&mut *tx)
            .await;
        assert!(denied.is_err(), "meetcal_api must not update other columns");
        tx.rollback().await.unwrap();
    }

    // Remove both links and confirm they're null again.
    for command in ["/meets-remove-pdf", "/meets-remove-map"] {
        let body =
            serde_urlencoded::to_string([("command", command), ("text", &format!("\"{meet}\""))])
                .unwrap();
        let resp = post_signed(&client, &cmd_url, &body).await;
        assert_eq!(resp.status(), 200);
        assert!(
            resp.json::<serde_json::Value>().await.unwrap()["text"]
                .as_str()
                .unwrap()
                .contains("Removed the")
        );
    }
    let details: serde_json::Value = reqwest::get(&details_url)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(details["venue_map_pdf_url"].is_null());
    assert!(details["venue_map_apple_url"].is_null());

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

    // --- an unsafe run id never becomes a file --------------------------
    let decision_files = || std::fs::read_dir(tmp.join("decisions")).unwrap().count();
    for bad_run_id in ["../escaped", "a/b", "", &"x".repeat(201)] {
        let payload = serde_json::json!({
            "type": "block_actions",
            "user": {"id": "U1", "username": "tester"},
            "channel": {"id": "C1"},
            "actions": [{"action_id": "meet_approve", "value": bad_run_id}]
        })
        .to_string();
        let body = serde_urlencoded::to_string([("payload", payload)]).unwrap();
        let resp = post_signed(&client, &int_url, &body).await;
        assert_eq!(resp.status(), 200, "Slack still gets an ack");
    }
    assert_eq!(decision_files(), 1, "only the valid decision exists");
    assert!(!tmp.join("escaped.json").exists());

    // --- second server: channel + user allowlists ------------------------
    // SAFETY: the first server keeps its already-parsed config; this only
    // affects the server spawned next.
    unsafe {
        std::env::set_var("SLACK_MEET_AUTOMATION_CHANNEL", "C-allowed");
        std::env::set_var("MEET_AUTOMATION_SLACK_ALLOWED_USERS", "U-allowed");
    }
    let app = support::spawn_test_app().await;
    let cmd_url = format!("{}/scrapers/slack/commands", app.address);
    let int_url = format!("{}/scrapers/slack/interactions", app.address);

    let command = |channel: &str, user: &str| {
        serde_urlencoded::to_string([
            ("command", "/meet-list"),
            ("text", ""),
            ("channel_id", channel),
            ("user_id", user),
        ])
        .unwrap()
    };
    let reply = |resp: reqwest::Response| async move {
        resp.json::<serde_json::Value>().await.unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string()
    };

    let text = reply(post_signed(&client, &cmd_url, &command("C-other", "U-allowed")).await).await;
    assert!(text.contains("aren't enabled in this channel"), "{text}");
    let text = reply(post_signed(&client, &cmd_url, &command("C-allowed", "U-other")).await).await;
    assert!(text.contains("not authorized"), "{text}");
    let text =
        reply(post_signed(&client, &cmd_url, &command("C-allowed", "U-allowed")).await).await;
    assert!(text.contains("No meet pages"), "{text}");

    // Button clicks honour the same allowlists: wrong channel or user records
    // nothing; the allowed pair does.
    let click = |channel: &str, user: &str, run_id: &str| {
        let payload = serde_json::json!({
            "type": "block_actions",
            "user": {"id": user, "username": "tester"},
            "channel": {"id": channel},
            "actions": [{"action_id": "meet_approve", "value": run_id}]
        })
        .to_string();
        serde_urlencoded::to_string([("payload", payload)]).unwrap()
    };
    let resp = post_signed(
        &client,
        &int_url,
        &click("C-other", "U-allowed", "itest-run-2"),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert!(!tmp.join("decisions/itest-run-2.json").exists());
    let resp = post_signed(
        &client,
        &int_url,
        &click("C-allowed", "U-other", "itest-run-3"),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert!(!tmp.join("decisions/itest-run-3.json").exists());
    let resp = post_signed(
        &client,
        &int_url,
        &click("C-allowed", "U-allowed", "itest-run-4"),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert!(tmp.join("decisions/itest-run-4.json").exists());

    let _ = std::fs::remove_dir_all(&tmp);
}
