use app::routes::comp_data::{
    get_adaptive_records::AdaptiveRecords, get_intl_rankings::IntlRanking,
    get_national_rankings::NatRankings, get_qualifying_totals::QualifyingTotal,
    get_records::Record, get_standards::Standard, get_wso_list::WsoNames,
    get_wso_records::WsoRecord,
};

const API: &str = "https://api.meetcal.app";

#[tokio::test]
async fn success_get_records() {
    let url = format!("{API}/records?record_type=USAW&gender=Men&age_category=Senior");
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
    let url = format!("{API}/records");
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_standards() {
    let url = format!("{API}/standards?gender=Men&age_category=Senior");
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
    let url = format!("{API}/standards");
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_wsos() {
    let url = format!("{API}/wso");
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: WsoNames = response.json().await.unwrap();

    assert!(!body.wsos.is_empty());
}

#[tokio::test]
async fn fail_get_wsos() {
    let url = format!("{API}/wsos");
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_wso_records() {
    let url = format!("{API}/wso-records?wso=Carolina&gender=Men&age_category=Senior");
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
    let url = format!("{API}/wso-records");
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_qualifying_totals() {
    let url = format!("{API}/qualifying-totals?event_name=Virus%20Finals&gender=Women&age_category=U11");
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
    let url = format!("{API}/qualifying-totals");
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_intl_rankings() {
    let url = format!("{API}/intl-rankings?meet=Worlds&gender=Women&age_category=Junior");
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
    let url = format!("{API}/intl-rankings");
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_nat_rankings() {
    let url = format!("{API}/nat-rankings?age_category=Open%20Men%27s%2060kg&federation=USAW");
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let _body: Vec<NatRankings> = response.json().await.unwrap();
}

#[tokio::test]
async fn fail_get_nat_rankings() {
    let url = format!("{API}/nat-rankings");
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_adaptive_records() {
    let url = format!("{API}/adaptive?exclude_federation=BWL&gender=Men");
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let _body: Vec<AdaptiveRecords> = response.json().await.unwrap();
}

#[tokio::test]
async fn fail_get_adaptive_records() {
    let url = format!("{API}/adaptive");
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
