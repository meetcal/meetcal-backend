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
async fn empty_search_query_is_rejected_for_strict_clients() {
    let app = support::spawn_test_app().await;
    let strict = client()
        .get(format!("{}/search?query=%20", app.address))
        .header("X-MeetCal-App", STRICT_CLIENT)
        .send()
        .await
        .unwrap();
    assert_eq!(strict.status(), 400);

    // A legacy client gets the empty payload, never a whole-table `%%` scan.
    let legacy = reqwest::get(format!("{}/search?query=%20", app.address))
        .await
        .unwrap();
    assert_eq!(legacy.status(), 200);
    let body: SearchResponse = legacy.json().await.unwrap();
    assert!(body.suggestions.is_empty());
    assert!(body.results.is_empty());
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

// ---------------------------------------------------------------------------
// Search date window is inclusive on both ends; rows carry their identity.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_end_date_is_exclusive() {
    let app = support::spawn_test_app().await;
    // Seed row is dated exactly 2025-06-01. Ranges are half-open, as every
    // app version sends a year: `YYYY-01-01` .. `YYYY+1-01-01`.
    let on_the_day: SearchResponse = reqwest::get(format!(
        "{}/search?query=Alexander%20Nordstrom&start_date=2025-06-01&end_date=2025-06-02",
        app.address
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(
        on_the_day.matched_name.as_deref(),
        Some("Alexander Nordstrom")
    );
    assert_eq!(on_the_day.results.len(), 1);
    assert_eq!(on_the_day.results[0].date, "2025-06-01");
    assert!(on_the_day.results[0].id > 0);
    assert_eq!(on_the_day.results[0].event_id, "event_2025");
    assert!(
        on_the_day.suggestions.is_empty(),
        "an exact match does not carry suggestions"
    );

    // An end date equal to the result's date excludes it.
    let day_before: SearchResponse = reqwest::get(format!(
        "{}/search?query=Alexander%20Nordstrom&start_date=2025-01-01&end_date=2025-06-01",
        app.address
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(day_before.results.is_empty());
    assert!(
        day_before
            .suggestions
            .contains(&"Alexander Nordstrom".to_string()),
        "no rows in range: suggestions are offered, {day_before:?}"
    );
}

#[tokio::test]
async fn result_rows_carry_id_and_event_id_everywhere() {
    let app = support::spawn_test_app().await;
    for path in [
        "/lifting-results/by-names?names=Alexander%20Nordstrom",
        "/lifting-results/recent?names=Alexander%20Nordstrom&cutoff_date=2025-01-01",
        "/lifting-results?meet=2025%20Test%20Meet",
    ] {
        let rows: Vec<serde_json::Value> = reqwest::get(format!("{}{path}", app.address))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!rows.is_empty(), "{path}");
        assert!(rows[0]["id"].is_i64(), "{path}: {}", rows[0]);
        assert_eq!(rows[0]["event_id"], "event_2025", "{path}");
    }
}

// ---------------------------------------------------------------------------
// `/lifting-results/by-names` bounds: latest_only and limit_per_name.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn by_names_can_be_bounded_per_name() {
    let app = support::spawn_test_app().await;
    let db = support::db_pool().await;
    let name = "Bounded History Lifter";
    sqlx::query("DELETE FROM lifting_results WHERE name = $1")
        .bind(name)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        r#"
        INSERT INTO lifting_results (convex_id, event_id, meet, date, name, age, body_weight,
                                     snatch1, snatch2, snatch3, snatch_best, cj1, cj2, cj3, cj_best,
                                     total, adaptive, federation)
        VALUES
            ('test-bounded-1', 'b1', 'Bounded Meet A', '2024-01-10', $1, 'Open Men''s 89kg',
             88, 90, 0, 0, 90, 110, 0, 0, 110, 200, false, 'USAW'),
            ('test-bounded-2', 'b2', 'Bounded Meet B', '2024-03-10', $1, 'Open Men''s 89kg',
             88, 92, 0, 0, 92, 112, 0, 0, 112, 204, false, 'USAW'),
            ('test-bounded-3', 'b3', 'Bounded Meet C', '2024-05-10', $1, 'Open Men''s 89kg',
             88, 94, 0, 0, 94, 114, 0, 0, 114, 208, false, 'USAW')
        "#,
    )
    .bind(name)
    .execute(&db)
    .await
    .unwrap();

    let fetch = |query: &str| {
        let url = format!(
            "{}/lifting-results/by-names?names=bounded%20history%20lifter{query}",
            app.address
        );
        async move {
            let response = reqwest::get(&url).await.unwrap();
            assert_eq!(response.status(), 200, "{url}");
            response.json::<Vec<LiftingResults>>().await.unwrap()
        }
    };

    let all = fetch("").await;
    let latest = fetch("&latest_only=true").await;
    let two = fetch("&limit_per_name=2").await;
    let posted = client()
        .post(format!("{}/lifting-results/by-names", app.address))
        .json(&serde_json::json!({ "names": [name], "limit_per_name": 1 }))
        .send()
        .await
        .unwrap();
    let posted_status = posted.status();
    let posted: Vec<LiftingResults> = posted.json().await.unwrap();

    sqlx::query("DELETE FROM lifting_results WHERE name = $1")
        .bind(name)
        .execute(&db)
        .await
        .unwrap();

    assert_eq!(all.len(), 3, "default is unbounded");
    assert_eq!(all[0].date, "2024-05-10", "newest first");
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].meet, "Bounded Meet C");
    assert_eq!(
        two.iter().map(|row| row.date.as_str()).collect::<Vec<_>>(),
        vec!["2024-05-10", "2024-03-10"]
    );
    assert_eq!(posted_status, 200);
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].total, 208.0);
}

#[tokio::test]
async fn by_names_limit_per_name_is_bounded() {
    let app = support::spawn_test_app().await;
    for query in ["limit_per_name=0", "limit_per_name=201"] {
        let response = reqwest::get(format!(
            "{}/lifting-results/by-names?names=Alexander%20Nordstrom&{query}",
            app.address
        ))
        .await
        .unwrap();
        assert_eq!(response.status(), 400, "{query}");
    }
    let posted = client()
        .post(format!("{}/lifting-results/by-names", app.address))
        .json(&serde_json::json!({ "names": ["Alexander Nordstrom"], "limit_per_name": 0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), 400);
}
