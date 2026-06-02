use app::{
    common::spawn_server,
    routes::comp_data::{
        get_records::Record, get_standards::Standard, get_wso_list::WsoNames,
        get_wso_records::WsoRecord,
    },
};

#[tokio::test]
async fn success_get_records() {
    let app = spawn_server::spawn_app().await;
    let url = format!(
        "{}/records?recordType=USAW&gender=men&ageCategory=senior",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<Record> = response.json().await.unwrap();

    assert!(!body.is_empty());
}

#[tokio::test]
async fn fail_get_records() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/records", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_standards() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/standards?gender=men&ageCategory=senior", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<Standard> = response.json().await.unwrap();

    assert!(!body.is_empty());
}

#[tokio::test]
async fn fail_get_standards() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/standards", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_wsos() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/wso", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: WsoNames = response.json().await.unwrap();

    assert!(!body.wsos.is_empty());
}

#[tokio::test]
async fn fail_get_wsos() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/wsos", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_wso_records() {
    let app = spawn_server::spawn_app().await;
    let url = format!(
        "{}/wso-records?wso=Carolina&gender=Men&ageCategory=Senior",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<WsoRecord> = response.json().await.unwrap();

    assert!(!body.is_empty());
}

#[tokio::test]
async fn fail_get_wso_records() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/wso-records", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
