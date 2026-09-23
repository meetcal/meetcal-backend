use app::routes::comp_data::{
    get_adaptive_records::AdaptiveRecords, get_intl_rankings::IntlRanking,
    get_national_rankings::NatRankings, get_qualifying_totals::QualifyingTotal,
    get_records::Record, get_standards::Standard, get_wso_records::WsoRecord,
};

mod support;

#[tokio::test]
async fn success_get_records() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/records", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<Record> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| {
        row.record_type == "USAW" && row.gender == "Men" && row.age_category == "Senior"
    }));
}

#[tokio::test]
async fn fail_get_records() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/record", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_standards() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/standards", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<Standard> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(
        body.iter()
            .all(|row| row.gender == "Men" && row.age_category == "Senior")
    );
}

#[tokio::test]
async fn fail_get_standards() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/standard", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_wsos() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/wso/", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<String> = response.json().await.unwrap();

    assert!(!body.is_empty());
}

#[tokio::test]
async fn success_get_wsos_without_trailing_slash() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/wso", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<String> = response.json().await.unwrap();

    assert!(!body.is_empty());
}

#[tokio::test]
async fn fail_get_wsos() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/wsos", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_wso_records() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/data/wso/records?wso=Carolina&gender=Men&age_category=Senior",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<WsoRecord> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| {
        row.wso == "Carolina" && row.gender == "Men" && row.age_category == "Senior"
    }));
}

#[tokio::test]
async fn fail_get_wso_records() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/wso/records", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_wso_records_with_wso_only() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/wso/records?wso=Carolina", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<WsoRecord> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| row.wso == "Carolina"));
}

#[tokio::test]
async fn success_get_wso_age_groups() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/wso/age-groups?wso=Carolina", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<String> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().any(|age_group| age_group == "Senior"));
}

#[tokio::test]
async fn success_get_qualifying_totals() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/qualifying-totals", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<QualifyingTotal> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| {
        row.event_name == "Virus Finals" && row.gender == "Women" && row.age_category == "U11"
    }));
}

#[tokio::test]
async fn fail_get_qualifying_totals() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/qualifying-total", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_intl_rankings() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/intl-rankings", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<IntlRanking> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| {
        row.meet == "Worlds" && row.gender == "Women" && row.age_category == "Junior"
    }));
}

#[tokio::test]
async fn fail_get_intl_rankings() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/intl-ranking", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_nat_rankings() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/data/nat-rankings?age_category=Open%20Men%27s%2060kg&federation=USAW",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let _body: Vec<NatRankings> = response.json().await.unwrap();
}

#[tokio::test]
async fn fail_get_nat_rankings() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/nat-rankings", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_adaptive_records() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/data/adaptive?exclude_federation=BWL&gender=Men",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let _body: Vec<AdaptiveRecords> = response.json().await.unwrap();
}

#[tokio::test]
async fn fail_get_adaptive_records() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/adaptive", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

// Client version gate: a blank `wso` is `200 []` for legacy callers (shipped
// app builds) and `400` once the client declares 6.2.0+.

#[tokio::test]
async fn blank_wso_is_empty_for_legacy_and_400_for_strict_clients() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    for path in ["/data/wso/records?wso=", "/data/wso/age-groups?wso=%20"] {
        let legacy = client
            .get(format!("{}{path}", app.address))
            .send()
            .await
            .unwrap();
        assert_eq!(legacy.status(), 200, "{path} legacy");
        let body: Vec<serde_json::Value> = legacy.json().await.unwrap();
        assert!(body.is_empty(), "{path} legacy body");

        let strict = client
            .get(format!("{}{path}", app.address))
            .header("X-MeetCal-App", "6.2.0")
            .send()
            .await
            .unwrap();
        assert_eq!(strict.status(), 400, "{path} strict");
        let body: serde_json::Value = strict.json().await.unwrap();
        assert_eq!(body["error"], "wso is required");
    }
}

#[tokio::test]
async fn reference_data_carries_cache_headers_and_revalidates() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    for path in [
        "/data/records",
        "/data/standards",
        "/data/qualifying-totals",
        "/data/intl-rankings",
    ] {
        let first = client
            .get(format!("{}{path}", app.address))
            .send()
            .await
            .unwrap();
        assert_eq!(first.status(), 200, "{path}");
        assert_eq!(
            first.headers().get(reqwest::header::CACHE_CONTROL).unwrap(),
            "public, max-age=300",
            "{path}"
        );
        let etag = first
            .headers()
            .get(reqwest::header::ETAG)
            .unwrap_or_else(|| panic!("{path} carries an ETag"))
            .clone();
        let revalidate = client
            .get(format!("{}{path}", app.address))
            .header(reqwest::header::IF_NONE_MATCH, &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(revalidate.status(), 304, "{path}");
    }
}

#[tokio::test]
async fn nat_rankings_year_is_validated_for_strict_clients() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let url = format!(
        "{}/data/nat-rankings-year?age_category=Open%20Men%27s%2060kg&federation=USAW&year=20x5",
        app.address
    );
    let legacy = client.get(&url).send().await.unwrap();
    assert_eq!(legacy.status(), 200);
    assert!(
        legacy
            .json::<Vec<serde_json::Value>>()
            .await
            .unwrap()
            .is_empty()
    );

    let strict = client
        .get(&url)
        .header("X-MeetCal-App", "6.2.0")
        .send()
        .await
        .unwrap();
    assert_eq!(strict.status(), 400);
    let body: serde_json::Value = strict.json().await.unwrap();
    assert_eq!(body["error"], "year must be a four-digit year");

    let valid: Vec<serde_json::Value> = client
        .get(format!(
            "{}/data/nat-rankings-year?age_category=Open%20Men%27s%2060kg&federation=USAW&year=2025",
            app.address
        ))
        .header("X-MeetCal-App", "6.2.0")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(valid.len(), 1);
    assert_eq!(valid[0]["name"], "Alexander Nordstrom");
}

#[tokio::test]
async fn adaptive_records_filter_season_and_gender_in_sql() {
    let app = support::spawn_test_app().await;
    let men: Vec<AdaptiveRecords> = reqwest::get(format!(
        "{}/data/adaptive?exclude_federation=BWL&gender=Men",
        app.address
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(men.len(), 1, "{men:?}");
    assert_eq!(men[0].weight_class, "85");
    assert_eq!(men[0].total, 90.0);

    let women: Vec<AdaptiveRecords> = reqwest::get(format!(
        "{}/data/adaptive?exclude_federation=BWL&gender=Women",
        app.address
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(women.is_empty(), "{women:?}");

    let next_season: Vec<AdaptiveRecords> = reqwest::get(format!(
        "{}/data/adaptive?exclude_federation=BWL&gender=Men&season=2027",
        app.address
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(next_season.is_empty(), "{next_season:?}");

    let bad_season = reqwest::get(format!(
        "{}/data/adaptive?exclude_federation=BWL&gender=Men&season=soon",
        app.address
    ))
    .await
    .unwrap();
    assert_eq!(bad_season.status(), 400);
}
