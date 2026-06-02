pub mod common;
pub mod error;
pub mod routes;

use axum::{Router, routing::get};
use convex::ConvexClient;
pub use error::AppError;
use routes::{
    clubs::get_all_clubs::get_all_clubs,
    comp_data::{
        get_records::get_records, get_standards::get_standards, get_wso_list::get_wso_list,
        get_wso_records::get_wso_records,
    },
    meets::{
        get_all_meets::list_meets_next_3months, get_athletes_by_meet::get_athletes_by_meet,
        get_meet_details::get_meet_details, get_meet_schedule::get_meet_schedule,
    },
};
use std::path::PathBuf;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;

#[derive(Clone)]
pub struct AppState {
    pub convex: ConvexClient,
}

pub fn load_env() {
    let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env");
    dotenvy::from_path(env_path).expect("failed to load meetcal-backend/.env");
}

pub async fn run(listener: TcpListener) {
    let convex_url =
        std::env::var("CONVEX_URL").expect("CONVEX_URL must be set in meetcal-backend/.env");

    let convex = ConvexClient::new(&convex_url)
        .await
        .expect("failed to connect to Convex");

    let app = Router::new()
        .route("/meets", get(list_meets_next_3months))
        .route("/meets/{name}", get(get_meet_details))
        .route("/meets/schedule/{name}", get(get_meet_schedule))
        .route("/meets/athletes/{name}", get(get_athletes_by_meet))
        .route("/clubs", get(get_all_clubs))
        .route("/records", get(get_records))
        .route("/wso", get(get_wso_list))
        .route("/wso-records", get(get_wso_records))
        .route("/standards", get(get_standards))
        .layer(CompressionLayer::new())
        .with_state(AppState { convex });

    axum::serve(listener, app).await.unwrap();
}
