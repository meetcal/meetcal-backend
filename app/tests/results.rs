use app::routes::lifting_results::get_results_current_year::YearBests;
use app::routes::results::search::SearchResponse;
use app::routes::results::types::LiftingResults;

mod support;

#[tokio::test]
async fn success_search() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/search?query=Alexander%20Nordstrom&start_date=2025-01-01&end_date=2025-12-31",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);

    let body: SearchResponse = response.json().await.unwrap();
    assert_eq!(body.matched_name.as_deref(), Some("Alexander Nordstrom"));
    assert!(!body.results.is_empty());
}

#[tokio::test]
async fn success_search_partial() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/search?query=Alexan&start_date=2025-01-01&end_date=2025-12-31",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);

    let body: SearchResponse = response.json().await.unwrap();
    assert!(!body.suggestions.is_empty());
    assert!(!body.results.is_empty());
}

#[tokio::test]
async fn fail_search() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/search", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_lifting_results_recent() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/lifting-results/recent?names=Adaptive%20Test%20Athlete&cutoff_date=2025-01-01",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<LiftingResults> = response.json().await.unwrap();

    assert_eq!(body.len(), 1);
    assert_eq!(body[0].name, "Adaptive Test Athlete");
    assert_eq!(body[0].total, 90.0);
}

#[tokio::test]
async fn success_get_lifting_results_recent_csv_names() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/lifting-results/recent?names=Adaptive%20Test%20Athlete,Alexander%20Nordstrom&cutoff_date=2025-01-01",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<LiftingResults> = response.json().await.unwrap();

    assert!(body.iter().any(|row| row.name == "Adaptive Test Athlete"));
}

#[tokio::test]
async fn fail_get_lifting_results_recent() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/lifting-results/recent", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_lifting_results_year_bests() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/lifting-results/year?name=Adaptive%20Test%20Athlete&cutoff_date=2025-01-01",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: YearBests = response.json().await.unwrap();

    assert_eq!(body.best_snatch, 40.0);
    assert_eq!(body.best_cj, 50.0);
    assert_eq!(body.best_total, 90.0);
}

#[tokio::test]
async fn fail_get_lifting_results_year_bests() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/lifting-results/year", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
