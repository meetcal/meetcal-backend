use app::routes::clubs::{get_all_clubs::Clubs, get_athletes_by_club::ClubsAthletes};

mod support;

#[tokio::test]
async fn success_get_all_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/clubs", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Clubs = response.json().await.unwrap();

    assert!(!body.names.is_empty());
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
async fn fail_get_athletes_by_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/clubs/athletes", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
