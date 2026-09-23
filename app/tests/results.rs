use app::routes::lifting_results::get_results_current_year::YearBests;
use app::routes::results::search::SearchResponse;
use app::routes::results::types::LiftingResults;
use std::collections::BTreeMap;

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

// Seed data stores "Alexander Nordstrom"; these queries pass differing case and
// extra whitespace to confirm name matching is normalized on the backend.

#[tokio::test]
async fn recent_results_match_name_case_insensitively() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/lifting-results/recent?names=alexander%20%20nordstrom&cutoff_date=2025-01-01",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);

    let body: Vec<LiftingResults> = response.json().await.unwrap();
    assert!(
        body.iter().any(|row| row.name == "Alexander Nordstrom"),
        "expected normalized name match, got {body:?}"
    );
}

#[tokio::test]
async fn by_names_match_name_case_insensitively() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/lifting-results/by-names?names=ALEXANDER%20NORDSTROM",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);

    let body: Vec<LiftingResults> = response.json().await.unwrap();
    assert!(body.iter().any(|row| row.name == "Alexander Nordstrom"));
}

#[tokio::test]
async fn year_best_matches_name_case_insensitively() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/lifting-results/year?name=alexander%20nordstrom&cutoff_date=2025-01-01",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);

    let body: YearBests = response.json().await.unwrap();
    assert_eq!(body.best_total, 230.0);
}

#[tokio::test]
async fn bests_keep_requested_name_key_when_case_differs() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/lifting-results/bests?names=alexander%20nordstrom&cutoff_date=2025-01-01",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);

    // Response stays keyed by the requested name even though the DB row differs in case.
    let body: BTreeMap<String, YearBests> = response.json().await.unwrap();
    let bests = body
        .get("alexander nordstrom")
        .expect("response keyed by requested name");
    assert_eq!(bests.best_total, 230.0);
}

#[tokio::test]
async fn search_matches_name_case_insensitively() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/search?query=alexander%20nordstrom&start_date=2025-01-01&end_date=2025-12-31",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);

    let body: SearchResponse = response.json().await.unwrap();
    assert!(!body.results.is_empty());
}

#[tokio::test]
async fn empty_names_are_rejected() {
    let app = support::spawn_test_app().await;
    let response = reqwest::get(format!("{}/lifting-results/by-names?names=", app.address))
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn oversized_name_lists_are_rejected() {
    let app = support::spawn_test_app().await;
    let names = (0..101)
        .map(|index| format!("name-{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let response = reqwest::get(format!(
        "{}/lifting-results/by-names?names={names}",
        app.address
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn empty_search_query_is_rejected() {
    let app = support::spawn_test_app().await;
    let response = reqwest::get(format!("{}/search?query=%20", app.address))
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn wildcard_search_query_does_not_match_every_name() {
    let app = support::spawn_test_app().await;
    // `%` is a LIKE wildcard. Unescaped it matched every seeded athlete, so the
    // endpoint answered a one-character query with the whole table.
    let response = reqwest::get(format!(
        "{}/search?query=%25&start_date=2025-01-01&end_date=2025-12-31",
        app.address
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), 200);

    let body: SearchResponse = response.json().await.unwrap();
    assert!(
        body.suggestions.is_empty(),
        "no seeded name contains a literal `%`, got {:?}",
        body.suggestions
    );
    assert!(
        body.results.is_empty(),
        "no seeded name contains a literal `%`, got {:?}",
        body.results
    );
}

#[tokio::test]
async fn underscore_search_query_does_not_match_every_name() {
    let app = support::spawn_test_app().await;
    // `_` is LIKE's single-character wildcard: `%_%` matched every non-empty name.
    let response = reqwest::get(format!("{}/search?query=_", app.address))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let body: SearchResponse = response.json().await.unwrap();
    assert!(
        body.suggestions.is_empty(),
        "no seeded name contains a literal `_`, got {:?}",
        body.suggestions
    );
}

// ---------------------------------------------------------------------------
// Client version gate + POST name lists. Legacy callers (no `X-MeetCal-App`)
// keep master's behaviour; a 6.2.0+ client opts into fail-closed validation.
// ---------------------------------------------------------------------------

const STRICT_CLIENT: &str = "6.2.0";
const LEGACY_CLIENT: &str = "6.1.0";

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test]
async fn legacy_clients_get_the_default_window_without_a_cutoff() {
    let app = support::spawn_test_app().await;
    for path in [
        "/lifting-results/recent?names=Adaptive%20Test%20Athlete",
        "/lifting-results/year?name=Adaptive%20Test%20Athlete",
        "/lifting-results/bests?names=Adaptive%20Test%20Athlete",
    ] {
        for version in [None, Some(LEGACY_CLIENT)] {
            let mut request = client().get(format!("{}{path}", app.address));
            if let Some(version) = version {
                request = request.header("X-MeetCal-App", version);
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), 200, "{path} version={version:?}");
        }
    }
}

#[tokio::test]
async fn strict_clients_must_send_a_valid_cutoff() {
    let app = support::spawn_test_app().await;
    for path in [
        "/lifting-results/recent?names=Adaptive%20Test%20Athlete",
        "/lifting-results/year?name=Adaptive%20Test%20Athlete",
        "/lifting-results/bests?names=Adaptive%20Test%20Athlete",
    ] {
        let missing = client()
            .get(format!("{}{path}", app.address))
            .header("X-MeetCal-App", STRICT_CLIENT)
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 400, "{path} missing cutoff");
        let body: serde_json::Value = missing.json().await.unwrap();
        assert_eq!(body["error"], "cutoff_date is required");

        let malformed = client()
            .get(format!("{}{path}&cutoff_date=2025-02-30", app.address))
            .header("X-MeetCal-App", STRICT_CLIENT)
            .send()
            .await
            .unwrap();
        assert_eq!(malformed.status(), 400, "{path} malformed cutoff");

        let valid = client()
            .get(format!("{}{path}&cutoff_date=2025-01-01", app.address))
            .header("X-MeetCal-App", STRICT_CLIENT)
            .send()
            .await
            .unwrap();
        assert_eq!(valid.status(), 200, "{path} valid cutoff");
    }
}

#[tokio::test]
async fn strict_clients_get_400_on_malformed_search_dates() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/search?query=Alexander%20Nordstrom&start_date=2025-13-01&end_date=2025-12-31",
        app.address
    );
    let legacy = client().get(&url).send().await.unwrap();
    assert_eq!(legacy.status(), 200);

    let strict = client()
        .get(&url)
        .header("X-MeetCal-App", STRICT_CLIENT)
        .send()
        .await
        .unwrap();
    assert_eq!(strict.status(), 400);
}

#[tokio::test]
async fn post_by_names_matches_get() {
    let app = support::spawn_test_app().await;
    let get: Vec<LiftingResults> = reqwest::get(format!(
        "{}/lifting-results/by-names?names=Alexander%20Nordstrom",
        app.address
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();

    let response = client()
        .post(format!("{}/lifting-results/by-names", app.address))
        .json(&serde_json::json!({ "names": ["Alexander Nordstrom"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let post: Vec<LiftingResults> = response.json().await.unwrap();

    assert!(!get.is_empty());
    assert_eq!(
        serde_json::to_value(&get).unwrap(),
        serde_json::to_value(&post).unwrap()
    );
}

#[tokio::test]
async fn post_recent_and_bests_accept_a_json_body() {
    let app = support::spawn_test_app().await;
    let body = serde_json::json!({
        "names": ["Adaptive Test Athlete"],
        "cutoff_date": "2025-01-01"
    });

    let recent = client()
        .post(format!("{}/lifting-results/recent", app.address))
        .header("X-MeetCal-App", STRICT_CLIENT)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(recent.status(), 200);
    let rows: Vec<LiftingResults> = recent.json().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].total, 90.0);

    let bests = client()
        .post(format!("{}/lifting-results/bests", app.address))
        .header("X-MeetCal-App", STRICT_CLIENT)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(bests.status(), 200);
    let by_name: BTreeMap<String, YearBests> = bests.json().await.unwrap();
    assert_eq!(by_name["Adaptive Test Athlete"].best_total, 90.0);
}

/// The reason POST exists: a comma inside a name is one name in a JSON array
/// but two names in the CSV query form. `/bests` echoes requested keys, which
/// makes the difference observable.
#[tokio::test]
async fn post_keeps_a_comma_inside_a_name_where_get_splits_it() {
    let app = support::spawn_test_app().await;

    let get: BTreeMap<String, YearBests> = reqwest::get(format!(
        "{}/lifting-results/bests?names=Nordstrom,%20Alexander&cutoff_date=2025-01-01",
        app.address
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(
        get.keys().collect::<Vec<_>>(),
        vec!["Alexander", "Nordstrom"],
        "GET splits on the comma"
    );

    let post: BTreeMap<String, YearBests> = client()
        .post(format!("{}/lifting-results/bests", app.address))
        .json(&serde_json::json!({
            "names": ["Nordstrom, Alexander"],
            "cutoff_date": "2025-01-01"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        post.keys().collect::<Vec<_>>(),
        vec!["Nordstrom, Alexander"],
        "POST keeps the name whole"
    );
}

#[tokio::test]
async fn post_name_lists_fail_closed_on_empty_and_oversized() {
    let app = support::spawn_test_app().await;
    let empty = client()
        .post(format!("{}/lifting-results/by-names", app.address))
        .json(&serde_json::json!({ "names": ["", "   "] }))
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), 400);

    let too_many: Vec<String> = (0..=app::common::query::MAX_NAME_LIST_LEN)
        .map(|index| format!("Athlete {index}"))
        .collect();
    let oversized = client()
        .post(format!("{}/lifting-results/by-names", app.address))
        .json(&serde_json::json!({ "names": too_many }))
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), 400);
}
