//! The API as production runs it: every query as `meetcal_api`, the
//! least-privileged role with row-level security, not the `postgres`
//! superuser the other suites use. A missing grant or policy shows up here as
//! a 500 (or a refused write) instead of in production.

use serde_json::json;

mod support;

const MEET: &str =
    "2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness";

#[tokio::test]
async fn read_routes_work_as_the_api_role() {
    let app = support::spawn_test_app_as_api_role().await;
    let client = reqwest::Client::new();
    let paths = [
        "/health".to_string(),
        "/meets".to_string(),
        "/meets/completed".to_string(),
        format!("/meets/details?meet={MEET}"),
        format!("/meets/schedule?meet={MEET}"),
        format!("/meets/athletes?meet={MEET}"),
        format!("/meets/athletes-sessions?meet={MEET}"),
        format!("/meets/package?meet={MEET}&history_cutoff_date=2024-01-01"),
        format!("/lifting-results?meet={MEET}"),
        "/lifting-results/by-names?names=Alexander%20Nordstrom".to_string(),
        "/lifting-results/recent?names=Alexander%20Nordstrom&cutoff_date=2024-01-01".to_string(),
        "/lifting-results/year?name=Alexander%20Nordstrom&cutoff_date=2024-01-01".to_string(),
        "/search?query=Alexander%20Nordstrom&start_date=2025-01-01&end_date=2026-01-01".to_string(),
        "/search?query=Alexan".to_string(),
        "/clubs".to_string(),
        "/data/records".to_string(),
        "/data/standards".to_string(),
        "/data/qualifying-totals".to_string(),
        "/data/intl-rankings".to_string(),
        "/data/wso/".to_string(),
        "/data/wso/records?wso=Carolina".to_string(),
        "/data/adaptive?exclude_federation=BWL&gender=Men".to_string(),
    ];
    for path in paths {
        let response = client
            .get(format!("{}{path}", app.address))
            .header("X-MeetCal-App", "6.2.0")
            .send()
            .await
            .unwrap();
        let status = response.status();
        assert!(
            status.is_success(),
            "{path} answered {status} as meetcal_api: {}",
            response.text().await.unwrap_or_default()
        );
    }
}

#[tokio::test]
async fn saved_sessions_round_trip_as_the_api_role() {
    let app = support::spawn_test_app_as_api_role().await;
    let client = reqwest::Client::new();
    let user = "test-user-api-role";
    let url = format!("{}/users/me/saved-sessions/api-role-1", app.address);

    let put = client
        .put(&url)
        .bearer_auth(support::test_token(user))
        .json(&json!({ "meet": "API Role Meet", "session_number": 1.0, "platform": "Red" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        put.status(),
        200,
        "{}",
        put.text().await.unwrap_or_default()
    );

    let list = client
        .get(format!("{}/users/me/saved-sessions", app.address))
        .bearer_auth(support::test_token(user))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    assert!(list.text().await.unwrap().contains("api-role-1"));

    let delete = client
        .delete(&url)
        .bearer_auth(support::test_token(user))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), 200);

    let prefs = client
        .patch(format!("{}/users/me/preferences/auto-unsave", app.address))
        .bearer_auth(support::test_token(user))
        .json(&json!({ "enabled": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(prefs.status(), 200);
}

#[tokio::test]
async fn the_api_role_can_verify_the_migration_ledger() {
    // `main` runs this check with the production role before serving.
    let db = support::db_pool().await;
    let mut conn = db.acquire().await.unwrap();
    sqlx::query("SET ROLE meetcal_api")
        .execute(&mut *conn)
        .await
        .unwrap();
    let applied: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success")
            .fetch_all(&mut *conn)
            .await
            .expect("meetcal_api can read _sqlx_migrations");
    sqlx::query("RESET ROLE").execute(&mut *conn).await.unwrap();
    assert!(
        app::common::schema::missing_migrations(
            &app::common::schema::embedded_migration_versions(),
            &applied
        )
        .is_empty()
    );
}
