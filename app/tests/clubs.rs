use app::{common::spawn_server, routes::clubs::get_all_clubs::Clubs};

#[tokio::test]
async fn success_get_all_clubs() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/clubs", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Clubs = response.json().await.unwrap();

    assert!(!body.names.is_empty());
}

#[tokio::test]
async fn fail_get_all_clubs() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/club", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
