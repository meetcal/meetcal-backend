use app::routes::meets::{
    get_sessions_for_athletes::SessionsAthletes,
    types::{Athlete, MeetSchedule, Meets},
};
use serde_json::Value;

mod support;

#[tokio::test]
async fn success_get_all_meets() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<Meets> = response.json().await.unwrap();

    assert!(!body.is_empty());
}

#[tokio::test]
async fn fail_get_all_meets() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meet", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_meet_details() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/meets/details?meet=2026%20Ohio%20WSO%20Championships",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Meets = response.json().await.unwrap();

    assert_eq!(body.name, "2026 Ohio WSO Championships");
}

#[tokio::test]
async fn fail_get_meet_details() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_athletes_by_meet() {
    let app = support::spawn_test_app().await;
    let meet = "2026 USA Weightlifting National Championships, Powered by Rogue Fitness";
    let url = format!(
        "{}/meets/athletes?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<Athlete> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| row.meet == meet));
}

#[tokio::test]
async fn fail_get_athletes_by_meet() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/athletes", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_meet_schedule() {
    let app = support::spawn_test_app().await;
    let meet = "2026 USA Weightlifting National Championships, Powered by Rogue Fitness";
    let url = format!(
        "{}/meets/schedule?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<MeetSchedule> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| row.meet == meet));
}

#[tokio::test]
async fn fail_get_meet_schedule() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/schedule", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_sessions_for_athletes() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/meets/athletes-sessions?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<SessionsAthletes> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| row.session_number == Some(45.0)));
}

#[tokio::test]
async fn success_get_sessions_for_athletes_filtered() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/meets/athletes-sessions?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness&session_number=45&platform=Red",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<SessionsAthletes> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(
        body.iter()
            .all(|row| row.session_number == Some(45.0) && row.session_platform.as_deref() == Some("Red"))
    );
}

#[tokio::test]
async fn success_get_sessions_for_athletes_without_schedule() {
    let app = support::spawn_test_app().await;

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
    let db = sqlx::PgPool::connect(&database_url).await.unwrap();

    // An athlete registered before sessions are assigned must still appear
    // in the start list (LEFT JOIN), with null session fields.
    sqlx::query(
        r#"
        INSERT INTO athletes (convex_id, member_id, name, age, club, gender, weight_class, entry_total, meet)
        VALUES ('test-unassigned-athlete', '0', 'Test Unassigned', 25, 'Test Club', 'Male', '89', 200,
                '2026 USA Weightlifting National Championships, Powered by Rogue Fitness')
        ON CONFLICT (convex_id) DO NOTHING
        "#,
    )
    .execute(&db)
    .await
    .unwrap();

    let url = format!(
        "{}/meets/athletes-sessions?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Vec<SessionsAthletes> = response.json().await.unwrap();

    sqlx::query("DELETE FROM athletes WHERE convex_id = 'test-unassigned-athlete'")
        .execute(&db)
        .await
        .unwrap();

    let unassigned = body
        .iter()
        .find(|row| row.name == "Test Unassigned")
        .expect("athlete without a session must be returned");
    assert_eq!(unassigned.session_number, None);
    assert_eq!(unassigned.date, None);
}

#[tokio::test]
async fn fail_get_sessions_for_athletes() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/athletes-sessions", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_meet_package() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/meets/package?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness&history_cutoff_date=2024-01-01",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Value = response.json().await.unwrap();

    assert_eq!(
        body["meet"]["name"],
        "2026 USA Weightlifting National Championships, Powered by Rogue Fitness"
    );
    assert!(!body["schedule"].as_array().unwrap().is_empty());
    assert!(!body["athletes"].as_array().unwrap().is_empty());
    assert!(body["meet_results"].as_array().unwrap().is_empty());
    assert!(!body["attempt_estimates"].as_array().unwrap().is_empty());
    assert_eq!(body["attempt_estimates"][0]["session_number"], 45.0);
    assert_eq!(body["attempt_estimates"][0]["platform"], "Red");
    assert_eq!(
        body["attempt_estimates"][0]["estimates"][0]["athlete_name"],
        "Kyle Schulman"
    );
    assert_eq!(
        body["attempt_estimates"][0]["estimates"][0]["snatch"]["attempts"],
        serde_json::json!([146.0, 149.0, 152.0])
    );
    assert_eq!(
        body["attempt_estimates"][0]["estimates"][0]["clean_and_jerk"]["attempts"],
        serde_json::json!([193.0, 197.0, 201.0])
    );
    assert_eq!(
        body["attempt_estimates"][0]["estimates"][0]["snatch"]["source"],
        "entry_total"
    );
    assert!(body["year_bests_by_name"].is_object());
    assert!(body["recent_results_by_name"].is_object());
}

#[tokio::test]
async fn fail_get_meet_package() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/package", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
