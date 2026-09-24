use app::{
    common::query::MAX_SAVED_SESSION_ATHLETE_NAMES,
    routes::users::{
        USER_WRITE_BODY_LIMIT,
        preferences::UserPreferencesResponse,
        saved_sessions::{
            DeleteSavedSessionResponse, DeleteSavedSessionsResponse,
            MAX_SAVED_SESSION_ATHLETE_NAME_LEN, MAX_SAVED_SESSION_ID_LEN,
            MAX_SAVED_SESSION_MEET_LEN, MAX_SAVED_SESSION_NOTES_LEN, MAX_SAVED_SESSIONS_PER_USER,
            SaveSessionResponse, SavedSessionsResponse,
        },
    },
};
use serde_json::{Value, json};
use sqlx::Acquire;

mod support;

#[tokio::test]
async fn success_saved_sessions_lifecycle() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let session_id = "2026-Nationals-45-Red";
    let base_url = format!("{}/users/me/saved-sessions", app.address);
    let session_url = format!("{base_url}/{session_id}");

    let save_response = client
        .put(&session_url)
        .bearer_auth(support::test_token("test-user-saved-sessions"))
        .json(&json!({
            "meet": "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
            "session_number": 45.0,
            "platform": "Red",
            "weight_class": "+110",
            "start_time": "08:00:00",
            "date": "2026-06-20",
            "notes": "test note",
            "athlete_names": ["Kyle Schulman"]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(save_response.status(), 200);

    let saved: SaveSessionResponse = save_response.json().await.unwrap();
    assert_eq!(saved.session_id, session_id);
    assert!(saved.updated_at > 0);

    let get_response = client
        .get(&base_url)
        .bearer_auth(support::test_token("test-user-saved-sessions"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_response.status(), 200);

    let sessions: SavedSessionsResponse = get_response.json().await.unwrap();
    let session = sessions
        .sessions
        .iter()
        .find(|session| session.session_id == session_id)
        .unwrap();

    assert_eq!(
        session.meet,
        "2026 USA Weightlifting National Championships, Powered by Rogue Fitness"
    );
    assert_eq!(session.session_number, 45.0);
    assert_eq!(session.platform, "Red");
    assert_eq!(session.athlete_names, vec!["Kyle Schulman".to_string()]);

    let delete_response = client
        .delete(&session_url)
        .bearer_auth(support::test_token("test-user-saved-sessions"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), 200);

    let deleted: DeleteSavedSessionResponse = delete_response.json().await.unwrap();
    assert!(deleted.deleted);
}

#[tokio::test]
async fn success_delete_saved_sessions_for_meet() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let base_url = format!("{}/users/me/saved-sessions", app.address);
    let session_url = format!("{base_url}/2026-Nationals-45-Red-Bulk");

    let save_response = client
        .put(&session_url)
        .bearer_auth(support::test_token("test-user-saved-sessions-bulk"))
        .json(&json!({
            "meet": "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
            "session_number": 45.0,
            "platform": "Red"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(save_response.status(), 200);

    let delete_response = client
        .delete(format!(
            "{base_url}?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness"
        ))
        .bearer_auth(support::test_token("test-user-saved-sessions-bulk"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), 200);

    let deleted: DeleteSavedSessionsResponse = delete_response.json().await.unwrap();
    assert!(deleted.deleted_count >= 1);
}

#[tokio::test]
async fn fail_saved_sessions_without_auth() {
    let app = support::spawn_test_app().await;
    let response = reqwest::get(format!("{}/users/me/saved-sessions", app.address))
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn success_get_default_preferences() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/users/me/preferences", app.address))
        .bearer_auth(support::test_token("test-user-preferences-default"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let body: UserPreferencesResponse = response.json().await.unwrap();
    assert!(!body.auto_unsave_started_sessions);
}

#[tokio::test]
async fn success_patch_auto_unsave_preferences() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let response = client
        .patch(format!("{}/users/me/preferences/auto-unsave", app.address))
        .bearer_auth(support::test_token("test-user-preferences-patch"))
        .json(&json!({ "enabled": true }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let body: UserPreferencesResponse = response.json().await.unwrap();
    assert!(body.auto_unsave_started_sessions);
}

#[tokio::test]
async fn fail_preferences_without_auth() {
    let app = support::spawn_test_app().await;
    let response = reqwest::get(format!("{}/users/me/preferences", app.address))
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn fail_preferences_with_forged_unsigned_token() {
    let app = support::spawn_test_app().await;
    let response = reqwest::Client::new()
        .get(format!("{}/users/me/preferences", app.address))
        .bearer_auth("e30.eyJzdWIiOiJhdHRhY2tlciJ9.forged")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn fail_preferences_with_expired_token() {
    let app = support::spawn_test_app().await;
    let response = reqwest::Client::new()
        .get(format!("{}/users/me/preferences", app.address))
        .bearer_auth(support::test_token_with(
            "test-user-expired",
            "https://clerk.test",
            "https://meetcal.app",
            -300,
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn fail_preferences_with_untrusted_authorized_party() {
    let app = support::spawn_test_app().await;
    let response = reqwest::Client::new()
        .get(format!("{}/users/me/preferences", app.address))
        .bearer_auth(support::test_token_with(
            "test-user-wrong-azp",
            "https://clerk.test",
            "https://evil.example",
            300,
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn fail_saved_session_with_empty_meet() {
    let app = support::spawn_test_app().await;
    let response = reqwest::Client::new()
        .put(format!(
            "{}/users/me/saved-sessions/empty-meet",
            app.address
        ))
        .bearer_auth(support::test_token("test-user-saved-sessions-empty"))
        .json(&json!({
            "meet": "  ",
            "session_number": 1.0,
            "platform": "Red"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
}

/// Native Clerk session tokens carry no `azp`; they must still be accepted.
#[tokio::test]
async fn success_token_without_azp_is_accepted() {
    let app = support::spawn_test_app().await;
    let response = reqwest::Client::new()
        .get(format!("{}/users/me/preferences", app.address))
        .bearer_auth(support::test_token_without_azp(
            "test-user-native",
            "https://clerk.test",
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

/// Omitting `azp` must not relax any other check: the issuer still has to match.
#[tokio::test]
async fn fail_token_without_azp_and_wrong_issuer() {
    let app = support::spawn_test_app().await;
    let response = reqwest::Client::new()
        .get(format!("{}/users/me/preferences", app.address))
        .bearer_auth(support::test_token_without_azp(
            "test-user-native-wrong-iss",
            "https://wrong-issuer.test",
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

async fn put_session(
    client: &reqwest::Client,
    app: &app::common::spawn_server::TestApp,
    user: &str,
    session_id: &str,
    body: Value,
) -> reqwest::Response {
    client
        .put(format!(
            "{}/users/me/saved-sessions/{session_id}",
            app.address
        ))
        .bearer_auth(support::test_token(user))
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// A 400 whose body names the cap: `{"error": ..., "max": N}`.
async fn assert_over_limit(response: reqwest::Response, max: usize) {
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert!(body["error"].is_string(), "{body}");
    assert_eq!(body["max"], json!(max), "{body}");
}

#[tokio::test]
async fn fail_saved_session_over_athlete_names_cap() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let user = "test-user-saved-sessions-many-names";
    let names =
        |count: usize| -> Vec<String> { (0..count).map(|i| format!("Athlete {i}")).collect() };

    // Exactly the cap is accepted.
    let response = put_session(
        &client,
        &app,
        user,
        "cap-names",
        json!({ "meet": "M", "session_number": 1.0, "platform": "Red",
                "athlete_names": names(MAX_SAVED_SESSION_ATHLETE_NAMES) }),
    )
    .await;
    assert_eq!(response.status(), 200);

    // One past it is refused, and the body says what the cap is.
    let response = put_session(
        &client,
        &app,
        user,
        "cap-names",
        json!({ "meet": "M", "session_number": 1.0, "platform": "Red",
                "athlete_names": names(MAX_SAVED_SESSION_ATHLETE_NAMES + 1) }),
    )
    .await;
    assert_over_limit(response, MAX_SAVED_SESSION_ATHLETE_NAMES).await;

    client
        .delete(format!("{}/users/me/saved-sessions", app.address))
        .bearer_auth(support::test_token(user))
        .send()
        .await
        .unwrap();
}

#[tokio::test]
async fn fail_saved_session_with_oversized_fields() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let user = "test-user-saved-sessions-oversized";
    let base = json!({ "meet": "M", "session_number": 1.0, "platform": "Red" });

    let response = put_session(
        &client,
        &app,
        user,
        &"s".repeat(MAX_SAVED_SESSION_ID_LEN + 1),
        base.clone(),
    )
    .await;
    assert_over_limit(response, MAX_SAVED_SESSION_ID_LEN).await;

    let mut body = base.clone();
    body["meet"] = json!("m".repeat(MAX_SAVED_SESSION_MEET_LEN + 1));
    assert_over_limit(
        put_session(&client, &app, user, "oversized", body).await,
        MAX_SAVED_SESSION_MEET_LEN,
    )
    .await;

    let mut body = base.clone();
    body["notes"] = json!("n".repeat(MAX_SAVED_SESSION_NOTES_LEN + 1));
    assert_over_limit(
        put_session(&client, &app, user, "oversized", body).await,
        MAX_SAVED_SESSION_NOTES_LEN,
    )
    .await;

    let mut body = base.clone();
    body["athlete_names"] = json!(["ok", "a".repeat(MAX_SAVED_SESSION_ATHLETE_NAME_LEN + 1)]);
    assert_over_limit(
        put_session(&client, &app, user, "oversized", body).await,
        MAX_SAVED_SESSION_ATHLETE_NAME_LEN,
    )
    .await;

    // Nothing above was stored.
    let sessions: SavedSessionsResponse = client
        .get(format!("{}/users/me/saved-sessions", app.address))
        .bearer_auth(support::test_token(user))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sessions.sessions.is_empty());
}

#[tokio::test]
async fn fail_saved_session_over_per_user_cap() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let user = "test-user-saved-sessions-full";
    let body = json!({ "meet": "M", "session_number": 1.0, "platform": "Red" });

    // Fill the account straight in the database (as the superuser, which
    // bypasses RLS) rather than through 500 HTTP round trips.
    let db = support::db_pool().await;
    sqlx::query(
        r#"
        INSERT INTO saved_sessions
            (convex_id, session_id, user_id, meet, session_number, platform, athlete_names, updated_at)
        SELECT 'saved_session:' || $1 || ':full-' || n, 'full-' || n, $1, 'M', n, 'Red', ARRAY[]::text[], 0
        FROM generate_series(1, $2) AS n
        "#,
    )
    .bind(user)
    .bind(MAX_SAVED_SESSIONS_PER_USER as i32)
    .execute(&db)
    .await
    .unwrap();

    // A new session is refused with the cap in the body...
    let response = put_session(&client, &app, user, "one-more", body.clone()).await;
    assert_over_limit(response, MAX_SAVED_SESSIONS_PER_USER).await;

    // ...but editing one the user already has still works.
    let response = put_session(&client, &app, user, "full-1", body).await;
    assert_eq!(response.status(), 200);

    let deleted: DeleteSavedSessionsResponse = client
        .delete(format!("{}/users/me/saved-sessions", app.address))
        .bearer_auth(support::test_token(user))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted.deleted_count, MAX_SAVED_SESSIONS_PER_USER as i64);
}

/// User B can neither see nor delete user A's sessions through the API.
#[tokio::test]
async fn fail_cross_user_saved_session_access() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let (user_a, user_b) = ("test-user-isolation-a", "test-user-isolation-b");
    let session_id = "isolation-1";
    let base_url = format!("{}/users/me/saved-sessions", app.address);
    let session_url = format!("{base_url}/{session_id}");
    let meet = "Isolation Meet";

    let response = put_session(
        &client,
        &app,
        user_a,
        session_id,
        json!({ "meet": meet, "session_number": 1.0, "platform": "Red" }),
    )
    .await;
    assert_eq!(response.status(), 200);

    // B's list does not include A's session.
    let sessions: SavedSessionsResponse = client
        .get(&base_url)
        .bearer_auth(support::test_token(user_b))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sessions.sessions.iter().all(|s| s.session_id != session_id));

    // B cannot delete it by id or by meet.
    let deleted: DeleteSavedSessionResponse = client
        .delete(&session_url)
        .bearer_auth(support::test_token(user_b))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!deleted.deleted);
    let deleted: DeleteSavedSessionsResponse = client
        .delete(format!("{base_url}?meet=Isolation%20Meet"))
        .bearer_auth(support::test_token(user_b))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted.deleted_count, 0);

    // A still has it, and can remove it.
    let sessions: SavedSessionsResponse = client
        .get(&base_url)
        .bearer_auth(support::test_token(user_a))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sessions.sessions.iter().any(|s| s.session_id == session_id));
    let deleted: DeleteSavedSessionResponse = client
        .delete(&session_url)
        .bearer_auth(support::test_token(user_a))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(deleted.deleted);
}

/// The handlers filter by `user_id`, but production connects as `meetcal_api`
/// and relies on row-level security as the backstop. Exercise that role
/// directly: with the request user set to B, A's rows are invisible and
/// undeletable even without a `user_id` predicate.
#[tokio::test]
async fn rls_isolates_saved_sessions_under_api_role() {
    let db = support::db_pool().await;
    let (user_a, user_b) = ("test-user-rls-a", "test-user-rls-b");
    sqlx::query("DELETE FROM saved_sessions WHERE user_id IN ($1, $2)")
        .bind(user_a)
        .bind(user_b)
        .execute(&db)
        .await
        .unwrap();

    let mut conn = db.acquire().await.unwrap();
    sqlx::query("SET ROLE meetcal_api")
        .execute(&mut *conn)
        .await
        .unwrap();
    let mut tx = conn.begin().await.unwrap();

    // A writes as A.
    app::routes::users::auth::set_request_user(&mut tx, user_a)
        .await
        .unwrap();
    sqlx::query(
        r#"
        INSERT INTO saved_sessions
            (convex_id, session_id, user_id, meet, session_number, platform, athlete_names, updated_at)
        VALUES ('saved_session:rls:a', 'rls-1', $1, 'M', 1, 'Red', ARRAY[]::text[], 0)
        "#,
    )
    .bind(user_a)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Now the connection is B: A's row is not there to read or delete.
    app::routes::users::auth::set_request_user(&mut tx, user_b)
        .await
        .unwrap();
    let visible = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM saved_sessions WHERE session_id = 'rls-1'",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(visible, 0);
    let deleted = sqlx::query("DELETE FROM saved_sessions WHERE session_id = 'rls-1'")
        .execute(&mut *tx)
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(deleted, 0);
    // And B cannot forge a row for A.
    let forged = sqlx::query(
        r#"
        INSERT INTO saved_sessions
            (convex_id, session_id, user_id, meet, session_number, platform, athlete_names, updated_at)
        VALUES ('saved_session:rls:forged', 'rls-forged', $1, 'M', 1, 'Red', ARRAY[]::text[], 0)
        "#,
    )
    .bind(user_a)
    .execute(&mut *tx)
    .await;
    assert!(
        forged.is_err(),
        "insert for another user must violate the policy"
    );

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn user_writes_answer_413_json_past_the_body_limit() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let user = "test-user-body-limit";
    let notes = "n".repeat(USER_WRITE_BODY_LIMIT);
    let oversized =
        json!({ "meet": "M", "session_number": 1.0, "platform": "Red", "notes": notes })
            .to_string();

    let response = client
        .put(format!("{}/users/me/saved-sessions/too-big", app.address))
        .bearer_auth(support::test_token(user))
        .header("content-type", "application/json")
        .body(oversized)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body, json!({ "error": "request body too large" }));

    let padded = format!(r#"{{"enabled":true{}}}"#, " ".repeat(USER_WRITE_BODY_LIMIT));
    let response = client
        .patch(format!("{}/users/me/preferences/auto-unsave", app.address))
        .bearer_auth(support::test_token(user))
        .header("content-type", "application/json")
        .body(padded)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body, json!({ "error": "request body too large" }));
}

#[tokio::test]
async fn a_saved_session_at_every_field_cap_fits_under_the_body_limit() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let user = "test-user-body-limit-max";
    // Four-byte characters everywhere: the worst case the limit is sized for.
    let wide = |count: usize| "\u{1F3CB}".repeat(count);
    let body = json!({
        "meet": wide(MAX_SAVED_SESSION_MEET_LEN),
        "session_number": 1.0,
        "platform": "Red",
        "notes": wide(MAX_SAVED_SESSION_NOTES_LEN),
        "athlete_names": (0..MAX_SAVED_SESSION_ATHLETE_NAMES)
            .map(|_| wide(MAX_SAVED_SESSION_ATHLETE_NAME_LEN))
            .collect::<Vec<_>>(),
    });
    let response = put_session(&client, &app, user, "max-fields", body).await;
    assert_eq!(response.status(), 200);

    client
        .delete(format!("{}/users/me/saved-sessions", app.address))
        .bearer_auth(support::test_token(user))
        .send()
        .await
        .unwrap();
}
