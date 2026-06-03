use app::{common::spawn_server, routes::results::types::LiftingResults};

#[tokio::test]
async fn success_search() {
    let app = spawn_server::spawn_app().await;
    let url = format!(
        "{}/search?query=Alexander%20Nordstrom&start_date=2025-01-01&end_date=2025-12-31",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);

    let _body: Vec<LiftingResults> = response.json().await.unwrap();
}

#[tokio::test]
async fn success_search_partial() {
    let app = spawn_server::spawn_app().await;
    let url = format!(
        "{}/search?query=Alexan&start_date=2025-01-01&end_date=2025-12-31",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);

    let _body: Vec<LiftingResults> = response.json().await.unwrap();
}

#[tokio::test]
async fn fail_search() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/search", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
