use app::routes::comp_data::{
    get_adaptive_records::AdaptiveRecords, get_intl_rankings::IntlRanking,
    get_national_rankings::NatRankings, get_qualifying_totals::QualifyingTotal,
    get_records::Record, get_standards::Standard, get_wso_list::WsoNames,
    get_wso_records::WsoRecord,
};

mod support;

#[tokio::test]
async fn success_get_records() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/data/records?record_type=USAW&gender=Men&age_category=Senior",
        app.address
    );
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
    let url = format!("{}/data/records", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_standards() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/data/standards?gender=Men&age_category=Senior",
        app.address
    );
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
    let url = format!("{}/data/standards", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_wsos() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/data/wso/", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: WsoNames = response.json().await.unwrap();

    assert!(!body.wsos.is_empty());
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
async fn success_get_qualifying_totals() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/data/qualifying-totals?event_name=Virus%20Finals&gender=Women&age_category=U11",
        app.address
    );
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
    let url = format!("{}/data/qualifying-totals", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_intl_rankings() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/data/intl-rankings?meet=Worlds&gender=Women&age_category=Junior",
        app.address
    );
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
    let url = format!("{}/data/intl-rankings", app.address);
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
