use app::routes::meets::{
    get_all_meets::MeetsList3Months,
    get_sessions_for_athletes::SessionsAthletes,
    types::{Athlete, MeetSchedule, Meets},
};

mod support;

#[tokio::test]
async fn success_get_all_meets() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: MeetsList3Months = response.json().await.unwrap();

    assert!(!body.names.is_empty());
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
    assert!(body.iter().all(|row| row.session_number == 45.0));
}

#[tokio::test]
async fn fail_get_sessions_for_athletes() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/athletes-sessions", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
