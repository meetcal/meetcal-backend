//! Per-client rate limits and server-wide load shedding, against a spawned
//! server with small limits.
//!
//! Every limited server refills at 1 token/s and holds exactly one [`COSTLY`]
//! request per anonymous bucket (two per keyed bucket). A spent bucket then
//! needs five seconds to take another, so the assertions hold however slowly
//! a loaded CI machine sends the requests in between.
use app::common::rate_limit::{MAX_ROUTE_COST, RateLimitSettings, SEARCH_ROUTE_COST};
use app::common::spawn_server::{TestApp, spawn_app_with_limits};
use reqwest::{Response, StatusCode};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

const SECRET: &str = "0123456789abcdef0123456789abcdef";
/// Same length and alphabet as [`SECRET`], one character off.
const WRONG_SECRET: &str = "0123456789abcdef0123456789abcdeX";
/// The costliest route: one request spends a whole [`BURST`].
const COSTLY: &str = "/search?query=Test";
const COSTLY_COST: u32 = SEARCH_ROUTE_COST;
/// A cheap route with no database work worth mentioning.
const CHEAP: &str = "/data/wso";
/// The smallest bucket the server accepts: one request to the costliest route.
const BURST: u32 = MAX_ROUTE_COST;

const _: () = assert!(COSTLY_COST == BURST);

fn limits(enforce: bool) -> RateLimitSettings {
    RateLimitSettings {
        enforce,
        ip_tokens_per_second: 1,
        ip_burst: BURST,
        key_tokens_per_second: 1,
        key_burst: BURST * 2,
        ..RateLimitSettings::default()
    }
}

async fn get(app: &TestApp, path: &str, headers: &[(&str, &str)]) -> Response {
    let mut request = reqwest::Client::new().get(format!("{}{path}", app.address));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().await.unwrap()
}

/// A costly request that must be served (whatever the handler answers).
async fn assert_served(app: &TestApp, headers: &[(&str, &str)]) {
    let response = get(app, COSTLY, headers).await;
    assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
}

/// A costly request that must be limited: `429`, JSON body, whole-second
/// `Retry-After`. Returns the `Retry-After` seconds.
async fn assert_limited(app: &TestApp, headers: &[(&str, &str)]) -> u64 {
    let response = get(app, COSTLY, headers).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers()["content-type"].to_str().unwrap(),
        "application/json"
    );
    let retry_after: u64 = response.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .expect("Retry-After is whole seconds");
    assert!(retry_after >= 1);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!({"error": "rate limited"})
    );
    retry_after
}

#[tokio::test]
async fn enforce_mode_answers_429_with_retry_after_past_the_burst() {
    let app = spawn_app_with_limits(limits(true), "").await;

    // One search spends the whole bucket; the next needs all of it back, so
    // Retry-After is the time to refill the search's cost.
    assert_served(&app, &[]).await;
    let retry_after = assert_limited(&app, &[]).await;
    assert!(
        (u64::from(COSTLY_COST) - 2..=u64::from(COSTLY_COST)).contains(&retry_after),
        "Retry-After follows the refill of the search's cost: {retry_after}"
    );
}

#[tokio::test]
async fn a_valid_key_gets_its_own_larger_budget() {
    let app = spawn_app_with_limits(limits(true), &format!("atlas:{SECRET}")).await;
    let keyed = [("X-MeetCal-Key", SECRET)];

    // Twice the anonymous burst.
    assert_served(&app, &keyed).await;
    assert_served(&app, &keyed).await;
    assert_limited(&app, &keyed).await;

    // The key's traffic never touched the address's bucket.
    assert_served(&app, &[]).await;
    assert_limited(&app, &[]).await;
}

#[tokio::test]
async fn an_invalid_key_is_anonymous_and_indistinguishable() {
    let app = spawn_app_with_limits(limits(true), &format!("atlas:{SECRET}")).await;

    // A wrong key is served like no key (no 401) and spends the address's
    // bucket, which a short or anonymous request then finds spent.
    assert_served(&app, &[("X-MeetCal-Key", WRONG_SECRET)]).await;
    assert_limited(&app, &[("X-MeetCal-Key", "nope")]).await;
    assert_limited(&app, &[]).await;

    // The real key still has its own budget.
    assert_served(&app, &[("X-MeetCal-Key", SECRET)]).await;
}

#[tokio::test]
async fn forwarded_for_is_honoured_from_a_trusted_peer_rightmost_entry_first() {
    let app = spawn_app_with_limits(limits(true), "").await;
    let client_a = [("X-Forwarded-For", "198.51.100.9, 203.0.113.1")];

    assert_served(&app, &client_a).await;
    assert_limited(&app, &client_a).await;

    // The rightmost entry is the client; entries to its left are ignored.
    assert_served(&app, &[("X-Forwarded-For", "203.0.113.1, 203.0.113.2")]).await;
    assert_limited(&app, &[("X-Forwarded-For", "203.0.113.2, 203.0.113.1")]).await;

    // Without the header the (loopback) peer is the client.
    assert_served(&app, &[]).await;
}

#[tokio::test]
async fn forwarded_for_is_ignored_when_trust_is_off() {
    let app = spawn_app_with_limits(
        RateLimitSettings {
            trust_forwarded_for: false,
            ..limits(true)
        },
        "",
    )
    .await;

    assert_served(&app, &[("X-Forwarded-For", "203.0.113.1")]).await;
    // A different forwarded address is still the same peer.
    assert_limited(&app, &[("X-Forwarded-For", "203.0.113.2")]).await;
}

#[tokio::test]
async fn forwarded_for_is_ignored_from_a_peer_that_is_not_a_trusted_proxy() {
    let app = spawn_app_with_limits(
        RateLimitSettings {
            trusted_proxies: "10.0.0.0/8".to_string(),
            ..limits(true)
        },
        "",
    )
    .await;

    assert_served(&app, &[("X-Forwarded-For", "203.0.113.1")]).await;
    assert_limited(&app, &[("X-Forwarded-For", "203.0.113.2")]).await;
}

#[tokio::test]
async fn ipv6_addresses_in_one_64_share_a_bucket() {
    let app = spawn_app_with_limits(limits(true), "").await;

    assert_served(&app, &[("X-Forwarded-For", "2001:db8:1:2::1")]).await;
    assert_limited(
        &app,
        &[("X-Forwarded-For", "2001:db8:1:2:ffff:ffff:ffff:ffff")],
    )
    .await;
    assert_served(&app, &[("X-Forwarded-For", "2001:db8:1:3::1")]).await;
}

#[tokio::test]
async fn health_is_exempt() {
    let app = spawn_app_with_limits(limits(true), "").await;

    assert_served(&app, &[]).await;
    assert_limited(&app, &[]).await;
    for _ in 0..BURST * 4 {
        assert_eq!(get(&app, "/health", &[]).await.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn shadow_mode_never_rejects() {
    let app = spawn_app_with_limits(limits(false), "").await;

    for _ in 0..BURST * 4 {
        let response = get(&app, COSTLY, &[]).await;
        assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().get("retry-after").is_none());
    }
}

/// The web client's origin; the API allows it cross-origin.
const WEB_ORIGIN: &str = "https://meetcal.app";

/// A browser can read `response`: it carries the CORS allow-origin for
/// [`WEB_ORIGIN`] and exposes `Retry-After`, which it also carries.
fn assert_browser_readable_with_retry_after(response: &Response) {
    assert!(response.headers().contains_key("retry-after"));
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        WEB_ORIGIN,
        "CORS must wrap the {} answer",
        response.status()
    );
    let exposed = response.headers()["access-control-expose-headers"]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        exposed.split(',').any(|name| name.trim() == "retry-after"),
        "{exposed}"
    );
}

#[tokio::test]
async fn a_429_carries_cors_headers_and_exposes_retry_after() {
    let app = spawn_app_with_limits(limits(true), "").await;
    let origin = [("Origin", WEB_ORIGIN)];

    assert_served(&app, &origin).await;
    let response = get(&app, COSTLY, &origin).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_browser_readable_with_retry_after(&response);
}

/// Polls `path` until it answers `status`, at most [`ATTEMPTS`] times.
async fn wait_for_status(
    app: &TestApp,
    path: &str,
    headers: &[(&str, &str)],
    status: StatusCode,
) -> Response {
    const ATTEMPTS: usize = 100;
    const PAUSE: Duration = Duration::from_millis(20);
    for _ in 0..ATTEMPTS {
        let response = get(app, path, headers).await;
        if response.status() == status {
            return response;
        }
        tokio::time::sleep(PAUSE).await;
    }
    panic!("{path} never answered {status}");
}

#[tokio::test]
async fn the_load_shedder_answers_503_when_the_in_flight_cap_is_full() {
    let app = spawn_app_with_limits(
        RateLimitSettings {
            max_in_flight: 1,
            ..RateLimitSettings::default()
        },
        "",
    )
    .await;

    // Hold the only slot: a POST whose body never finishes keeps its handler
    // waiting on the body.
    let host = app.address.trim_start_matches("http://");
    let mut stalled = TcpStream::connect(host).await.unwrap();
    stalled
        .write_all(
            format!(
                "POST /lifting-results/by-names HTTP/1.1\r\nHost: {host}\r\n\
                 Content-Type: application/json\r\nContent-Length: 64\r\n\r\n{{"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stalled.flush().await.unwrap();

    // Sent from the web origin: the 503 must be readable by a browser too.
    let origin = [("Origin", WEB_ORIGIN)];
    let shed = wait_for_status(&app, CHEAP, &origin, StatusCode::SERVICE_UNAVAILABLE).await;
    assert_eq!(shed.headers()["retry-after"], "1");
    assert_browser_readable_with_retry_after(&shed);
    assert_eq!(
        shed.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!({"error": "overloaded"})
    );

    // Health still answers while the cap is full.
    assert_eq!(get(&app, "/health", &[]).await.status(), StatusCode::OK);

    // Closing the stalled request frees its slot.
    drop(stalled);
    wait_for_status(&app, CHEAP, &[], StatusCode::OK).await;
}
