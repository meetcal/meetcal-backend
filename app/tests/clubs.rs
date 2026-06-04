use app::routes::clubs::get_all_clubs::Clubs;

const API: &str = "https://api.meetcal.app";

#[tokio::test]
async fn success_get_all_clubs() {
    let url = format!("{API}/clubs");
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Clubs = response.json().await.unwrap();

    assert!(!body.names.is_empty());
}

#[tokio::test]
async fn fail_get_all_clubs() {
    let url = format!("{API}/club");
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
