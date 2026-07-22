//! Integration tests for the referral rewards program.
//!
//! These exercise the HTTP surface + the qualification/minting state machine
//! against a live Postgres. They connect as the superuser (like the other
//! integration tests), so RLS is bypassed; the app's explicit `user_id` filters
//! and status guards are what's under test here. The RLS policies themselves are
//! validated structurally by applying the migrations.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

mod support;

const WEBHOOK_SECRET: &str = "test-webhook-secret";
const JOB_SECRET: &str = "test-job-secret";

/// Set the webhook/job secrets before spawning the app (ReferralConfig reads
/// them from the environment at spawn time). Same values across tests, so
/// concurrent set/read is harmless.
fn configure_secrets() {
    // SAFETY: tests only ever set these to the same constant values.
    unsafe {
        std::env::set_var("REVENUECAT_WEBHOOK_SECRET", WEBHOOK_SECRET);
        std::env::set_var("INTERNAL_JOB_SECRET", JOB_SECRET);
    }
}

/// Build an UNSIGNED Clerk-shaped JWT for `sub` (auth verification is disabled in
/// the test config, matching the existing users tests).
fn token(sub: &str) -> String {
    let payload = URL_SAFE_NO_PAD.encode(json!({ "sub": sub }).to_string());
    format!("e30.{payload}.sig")
}

fn uid(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/meetcal".to_string());
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect test db")
}

fn paid_event(event_id: &str, app_user_id: &str) -> Value {
    json!({
        "event": {
            "id": event_id,
            "type": "INITIAL_PURCHASE",
            "app_user_id": app_user_id,
            "store": "APP_STORE",
            "period_type": "NORMAL",
            "product_id": "com.meetcal.monthly",
        },
        "api_version": "1.0",
    })
}

/// A $0 NORMAL renewal = the promotional free month applied.
fn promo_renewal_event(event_id: &str, app_user_id: &str) -> Value {
    json!({
        "event": {
            "id": event_id,
            "type": "RENEWAL",
            "app_user_id": app_user_id,
            "store": "APP_STORE",
            "period_type": "NORMAL",
            "product_id": "com.meetcal.monthly",
            "price": 0,
            "price_in_purchased_currency": 0,
        },
        "api_version": "1.0",
    })
}

async fn mint_reward(db: &PgPool, user: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO reward_ledger (user_id, status, idempotency_key) \
         VALUES ($1, 'earned', $2) RETURNING id",
    )
    .bind(user)
    .bind(uid("mint"))
    .fetch_one(db)
    .await
    .unwrap()
}

async fn post_webhook(client: &reqwest::Client, addr: &str, auth: &str, body: &Value) -> u16 {
    client
        .post(format!("{addr}/webhooks/revenuecat"))
        .header("Authorization", auth)
        .json(body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn get_referral(client: &reqwest::Client, addr: &str, sub: &str) -> Value {
    client
        .get(format!("{addr}/users/me/referral"))
        .bearer_auth(token(sub))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn webhook_rejects_bad_auth() {
    configure_secrets();
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();

    let status = post_webhook(
        &client,
        &app.address,
        "wrong-secret",
        &paid_event(&uid("evt"), &uid("user")),
    )
    .await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn webhook_is_idempotent_on_duplicate_event_id() {
    configure_secrets();
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let db = pool().await;

    let event_id = uid("evt");
    let user = uid("user");
    let body = paid_event(&event_id, &user);

    let first = post_webhook(&client, &app.address, WEBHOOK_SECRET, &body).await;
    let second = post_webhook(&client, &app.address, WEBHOOK_SECRET, &body).await;
    assert_eq!(first, 200);
    assert_eq!(second, 200);

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM subscription_events WHERE event_id = $1")
            .bind(&event_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(count, 1, "duplicate event_id must only be stored once");
}

#[tokio::test]
async fn web_store_events_do_not_qualify() {
    configure_secrets();
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();

    let referrer = uid("referrer");
    let referred = uid("referred");

    // Referrer gets a code; referred redeems it (pending).
    let code = get_referral(&client, &app.address, &referrer).await["code"]
        .as_str()
        .unwrap()
        .to_string();
    redeem(&client, &app.address, &referred, &code).await;

    // A web-billing paid event must NOT promote the referral.
    let mut body = paid_event(&uid("evt"), &referred);
    body["event"]["store"] = json!("rc_billing");
    assert_eq!(
        post_webhook(&client, &app.address, WEBHOOK_SECRET, &body).await,
        200
    );

    let referral = get_referral(&client, &app.address, &referrer).await;
    assert_eq!(referral["pending_count"], 1, "should still be pending");
    assert_eq!(referral["qualifying_count"], 0);
}

async fn redeem(client: &reqwest::Client, addr: &str, sub: &str, code: &str) -> reqwest::Response {
    client
        .post(format!("{addr}/users/me/referral/redeem"))
        .bearer_auth(token(sub))
        .json(&json!({ "code": code }))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn redeem_rejections() {
    configure_secrets();
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();

    let alice = uid("alice");
    let bob = uid("bob");
    let carol = uid("carol");

    let alice_code = get_referral(&client, &app.address, &alice).await["code"]
        .as_str()
        .unwrap()
        .to_string();

    // Self-referral.
    let resp = redeem(&client, &app.address, &alice, &alice_code).await;
    assert_eq!(resp.status(), 400);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"],
        "self_referral"
    );

    // Unknown code.
    let resp = redeem(&client, &app.address, &bob, "ZZZZZZZZ").await;
    assert_eq!(resp.status(), 404);
    assert_eq!(resp.json::<Value>().await.unwrap()["error"], "invalid_code");

    // Valid redeem, then double-redeem.
    let resp = redeem(&client, &app.address, &carol, &alice_code).await;
    assert_eq!(resp.status(), 200);
    let resp = redeem(&client, &app.address, &carol, &alice_code).await;
    assert_eq!(resp.status(), 409);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"],
        "already_redeemed"
    );
}

/// Full lifecycle: 5 referred users pay, the hold clears, qualification runs,
/// one reward month is minted. Then a refund on a qualified referral reverses
/// the still-earned reward and does not double-mint.
#[tokio::test]
async fn qualification_minting_and_reversal() {
    configure_secrets();
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let db = pool().await;

    let referrer = uid("ref");
    let code = get_referral(&client, &app.address, &referrer).await["code"]
        .as_str()
        .unwrap()
        .to_string();

    // 5 referred users redeem + pay.
    let mut referred_users = Vec::new();
    for _ in 0..5 {
        let u = uid("rd");
        assert_eq!(redeem(&client, &app.address, &u, &code).await.status(), 200);
        assert_eq!(
            post_webhook(
                &client,
                &app.address,
                WEBHOOK_SECRET,
                &paid_event(&uid("evt"), &u)
            )
            .await,
            200
        );
        referred_users.push(u);
    }

    // All 5 should be qualifying now.
    let referral = get_referral(&client, &app.address, &referrer).await;
    assert_eq!(referral["qualifying_count"], 5);
    assert_eq!(referral["qualified_count"], 0);

    // Age the subscription starts past the 30-day hold.
    sqlx::query(
        "UPDATE referrals SET subscription_started_at = now() - interval '40 days' \
         WHERE referrer_user_id = $1",
    )
    .bind(&referrer)
    .execute(&db)
    .await
    .unwrap();

    // Run qualification: promote 5 -> qualified, mint 1 reward.
    let run = run_qualification(&client, &app.address, JOB_SECRET).await;
    assert_eq!(run.status(), 200);
    let run_body = run.json::<Value>().await.unwrap();
    assert_eq!(run_body["promoted"], 5);
    assert_eq!(run_body["minted"], 1);

    let referral = get_referral(&client, &app.address, &referrer).await;
    assert_eq!(referral["qualified_count"], 5);
    assert_eq!(referral["reward_months_available"], 1);
    assert_eq!(referral["rewards"].as_array().unwrap().len(), 1);
    assert_eq!(referral["rewards"][0]["status"], "earned");

    // Re-running qualification must NOT mint again (derived formula).
    let run2 = run_qualification(&client, &app.address, JOB_SECRET).await;
    assert_eq!(run2.json::<Value>().await.unwrap()["minted"], 0);

    // Refund one qualified referral -> disqualify + reverse the earned reward.
    let mut refund = paid_event(&uid("evt"), &referred_users[0]);
    refund["event"]["type"] = json!("REFUND");
    assert_eq!(
        post_webhook(&client, &app.address, WEBHOOK_SECRET, &refund).await,
        200
    );

    let referral = get_referral(&client, &app.address, &referrer).await;
    assert_eq!(referral["qualified_count"], 4);
    assert_eq!(
        referral["reward_months_available"], 0,
        "earned reward should be clawed back"
    );

    // The reward row is now reversed, not deleted.
    let reversed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM reward_ledger WHERE user_id = $1 AND status = 'reversed'",
    )
    .bind(&referrer)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(reversed, 1);
}

#[tokio::test]
async fn android_claim_returns_503_when_unconfigured() {
    // GOOGLE_PLAY_* is never configured in the test env, so this is deterministic
    // regardless of whether another test has configured Apple.
    configure_secrets();
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let db = pool().await;

    let user = uid("claimer");
    let reward_id = mint_reward(&db, &user).await;

    let resp = client
        .post(format!(
            "{}/users/me/rewards/{reward_id}/claim",
            app.address
        ))
        .bearer_auth(token(&user))
        .json(&json!({ "platform": "android" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"],
        "not_configured"
    );

    let status: String = sqlx::query_scalar("SELECT status FROM reward_ledger WHERE id = $1")
        .bind(reward_id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(status, "earned");
}

/// Configure Apple signing globally for the test binary (a runtime-generated
/// P-256 key). Only GOOGLE_PLAY_* is left unset, so the Android 503 test stays
/// deterministic.
fn configure_apple() {
    use p256::ecdsa::SigningKey;
    use p256::pkcs8::EncodePrivateKey;
    let pem = SigningKey::from_bytes(&[9u8; 32].into())
        .unwrap()
        .to_pkcs8_pem(Default::default())
        .unwrap()
        .to_string();
    // SAFETY: set to constant values before the app under test spawns.
    unsafe {
        std::env::set_var("APPLE_IAP_KEY_ID", "TESTKEY");
        std::env::set_var("APPLE_BUNDLE_ID", "app.meetcal");
        std::env::set_var("APPLE_IAP_PRIVATE_KEY", &pem);
        std::env::set_var("APPLE_IAP_OFFER_IDS", "com.meetcal.monthly=free_month");
    }
}

/// C1/C2: the iOS claim returns a flattened signature bundle but does NOT consume
/// the reward — it stays `earned` so a cancelled Apple purchase doesn't burn it.
#[tokio::test]
async fn ios_claim_leaves_reward_earned_and_returns_flattened_signature() {
    configure_secrets();
    configure_apple();
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let db = pool().await;

    let user = uid("ios");
    let reward_id = mint_reward(&db, &user).await;

    // The claim needs a known non-web product to attach the offer to.
    sqlx::query(
        "INSERT INTO subscription_events (event_id, app_user_id, store, type, period_type, product_id, event) \
         VALUES ($1, $2, 'APP_STORE', 'INITIAL_PURCHASE', 'NORMAL', 'com.meetcal.monthly', '{}'::jsonb)",
    )
    .bind(uid("evt"))
    .bind(&user)
    .execute(&db)
    .await
    .unwrap();

    let resp = client
        .post(format!(
            "{}/users/me/rewards/{reward_id}/claim",
            app.address
        ))
        .bearer_auth(token(&user))
        .json(&json!({ "platform": "ios" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.json::<Value>().await.unwrap();

    // Flattened, top-level fields (not nested under `.offer`).
    assert_eq!(body["status"], "earned");
    assert_eq!(body["platform"], "ios");
    assert_eq!(body["product_id"], "com.meetcal.monthly");
    assert_eq!(body["offer_id"], "free_month");
    assert_eq!(body["key_id"], "TESTKEY");
    assert!(!body["signature"].as_str().unwrap().is_empty());
    assert!(body["nonce"].is_string());
    assert!(body["timestamp"].is_number());
    assert!(
        body.get("offer").is_none(),
        "must not be nested under .offer"
    );

    // Reward is NOT consumed: still earned, with an audit timestamp.
    let (status, issued): (String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT status, ios_offer_issued_at FROM reward_ledger WHERE id = $1")
            .bind(reward_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(status, "earned");
    assert!(issued.is_some());
}

/// C1: iOS delivery is confirmed by the webhook. A promo-redemption event
/// delivers exactly one reward; a duplicate event delivers none.
#[tokio::test]
async fn webhook_promo_redemption_delivers_one_reward_idempotently() {
    configure_secrets();
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let db = pool().await;

    let user = uid("promo");
    // Two earned rewards so we can prove a duplicate event doesn't deliver a 2nd.
    let r1 = mint_reward(&db, &user).await;
    let _r2 = mint_reward(&db, &user).await;

    let event_id = uid("evt");
    let body = promo_renewal_event(&event_id, &user);

    // First delivery: one reward -> delivered.
    assert_eq!(
        post_webhook(&client, &app.address, WEBHOOK_SECRET, &body).await,
        200
    );
    // Duplicate event (same event_id): deduped, delivers none.
    assert_eq!(
        post_webhook(&client, &app.address, WEBHOOK_SECRET, &body).await,
        200
    );

    let delivered: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM reward_ledger WHERE user_id = $1 AND status = 'delivered'",
    )
    .bind(&user)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        delivered, 1,
        "exactly one reward delivered, duplicate is a no-op"
    );

    // The oldest reward was the one delivered, tagged with the event id.
    let (status, txn): (String, Option<String>) =
        sqlx::query_as("SELECT status, store_transaction_id FROM reward_ledger WHERE id = $1")
            .bind(r1)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(status, "delivered");
    assert_eq!(txn.as_deref(), Some(event_id.as_str()));

    // A DIFFERENT promo event delivers the second reward.
    let body2 = promo_renewal_event(&uid("evt"), &user);
    assert_eq!(
        post_webhook(&client, &app.address, WEBHOOK_SECRET, &body2).await,
        200
    );
    let delivered: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM reward_ledger WHERE user_id = $1 AND status = 'delivered'",
    )
    .bind(&user)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(delivered, 2);
}

async fn run_qualification(
    client: &reqwest::Client,
    addr: &str,
    secret: &str,
) -> reqwest::Response {
    client
        .post(format!("{addr}/internal/referrals/run-qualification"))
        .header("X-Internal-Job-Secret", secret)
        .send()
        .await
        .unwrap()
}
