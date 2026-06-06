pub mod common;
pub mod configuration;
pub mod error;
pub mod routes;

use crate::routes::{
    clubs::get_athletes_by_club::get_athletes_by_club,
    comp_data::{
        get_adaptive_records::get_adaptive_records, get_national_rankings::get_national_rankings,
    },
    meets::get_sessions_for_athletes::get_sessions_for_athletes,
    results::search::search_wrapped,
};
use axum::{Router, routing::get};
pub use error::AppError;
use routes::{
    clubs::get_all_clubs::get_all_clubs,
    comp_data::{
        get_intl_rankings::get_intl_rankings, get_qualifying_totals::get_qualifying_totals,
        get_records::get_records, get_standards::get_standards, get_wso_list::get_wso_list,
        get_wso_records::get_wso_records,
    },
    health::health,
    meets::{
        get_all_meets::list_meets_next_3months, get_athletes_by_meet::get_athletes_by_meet,
        get_meet_details::get_meet_details, get_meet_schedule::get_meet_schedule,
    },
};
use sqlx::PgPool;
use std::path::PathBuf;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}

pub fn load_env() {
    let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env");
    if env_path.exists() {
        dotenvy::from_path(env_path).expect("failed to load meetcal-backend/.env");
    }
}

pub async fn run(listener: TcpListener, db: PgPool) {
    let app = Router::new()
        .route("/health", get(health))
        .route("/meets", get(list_meets_next_3months))
        .route("/meets/details", get(get_meet_details))
        .route("/meets/schedule", get(get_meet_schedule))
        .route("/meets/athletes", get(get_athletes_by_meet))
        .route("/meets/athletes-sessions", get(get_sessions_for_athletes))
        .route("/clubs", get(get_all_clubs))
        .route("/clubs/athletes", get(get_athletes_by_club))
        .route("/data/records", get(get_records))
        .route("/data/wso/", get(get_wso_list))
        .route("/data/wso/records", get(get_wso_records))
        .route("/data/standards", get(get_standards))
        .route("/data/qualifying-totals", get(get_qualifying_totals))
        .route("/data/intl-rankings", get(get_intl_rankings))
        .route("/data/nat-rankings", get(get_national_rankings))
        .route("/data/adaptive", get(get_adaptive_records))
        .route("/search", get(search_wrapped))
        .layer(CompressionLayer::new())
        .with_state(AppState { db });

    axum::serve(listener, app).await.unwrap();
}
