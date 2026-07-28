use app::routes::clubs::get_athletes_by_club::ClubsAthletes;
use serde_json::Value;

mod support;

#[tokio::test]
async fn success_get_all_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/clubs", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<String> = response.json().await.unwrap();

    assert!(!body.is_empty());
}

#[tokio::test]
async fn fail_get_all_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/club", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_athletes_by_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/clubs/athletes?meet=2026%20Ohio%20WSO%20Championships&club=Columbus%20Weightlifting",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<ClubsAthletes> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| row.club == "Columbus Weightlifting"));
}

#[tokio::test]
async fn club_athletes_include_registrations_for_non_completed_meets() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/clubs/athletes?club=Vardanian%20Weightlifting",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<ClubsAthletes> = response.json().await.unwrap();

    assert_eq!(body.len(), 1);
    assert_eq!(body[0].name, "Kyle Schulman");
    assert_eq!(
        body[0].meet,
        "2026 USA Weightlifting National Championships, Powered by Rogue Fitness"
    );
}

#[tokio::test]
async fn fail_get_athletes_by_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/clubs/athletes", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_club_meet_stats() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/clubs/meet-stats?meet=2026%20Ohio%20WSO%20Championships&club=Columbus%20Weightlifting",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Value = response.json().await.unwrap();

    assert_eq!(body["total_athletes"], 1);
    assert_eq!(body["gold_medals"], 0);
    assert_eq!(body["silver_medals"], 0);
    assert_eq!(body["bronze_medals"], 0);
    assert_eq!(body["total_prs"], 0);
    assert_eq!(body["perfect_6_for_6"], 0);
    assert_eq!(body["snatch_make_rate"], 0);
    assert_eq!(body["cj_make_rate"], 0);
    assert_eq!(body["combined_make_rate"], 0);
    assert!(body["athlete_results"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn fail_get_club_meet_stats() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/clubs/meet-stats", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
