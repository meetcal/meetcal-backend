use app::routes::wsos::get_athletes_by_wso::WsoAthlete;

mod support;

#[tokio::test]
async fn gets_registrations_by_wso() {
    let app = support::spawn_test_app().await;
    let response = reqwest::get(format!("{}/wsos/athletes?wso=Ohio", app.address))
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<WsoAthlete> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| row.wso == "Ohio"));
    assert!(body.iter().any(|row| row.name == "Columbus Test Athlete"));
}

#[tokio::test]
async fn missing_wso_query_is_rejected() {
    let app = support::spawn_test_app().await;
    let response = reqwest::get(format!("{}/wsos/athletes", app.address))
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn unknown_wso_returns_an_empty_list() {
    let app = support::spawn_test_app().await;
    let response = reqwest::get(format!(
        "{}/wsos/athletes?wso=Definitely%20Not%20A%20WSO",
        app.address
    ))
    .await
    .unwrap();

    assert_eq!(response.status(), 200);
    assert!(response.json::<Vec<WsoAthlete>>().await.unwrap().is_empty());
}
