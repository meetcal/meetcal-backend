use app::routes::users::{
    preferences::UserPreferencesResponse,
    saved_sessions::{
        DeleteSavedSessionResponse, DeleteSavedSessionsResponse, SaveSessionResponse,
        SavedSessionsResponse,
    },
};
use serde_json::json;

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
