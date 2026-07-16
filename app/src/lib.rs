pub mod common;
pub mod configuration;
pub mod error;
pub mod routes;

use crate::routes::{
    clubs::{get_athletes_by_club::get_athletes_by_club, get_meet_stats::get_meet_stats},
    comp_data::{
        get_adaptive_records::get_adaptive_records, get_national_rankings::get_national_rankings,
    },
    lifting_results::{
        get_lifting_results::get_lifting_results,
        get_results_2yrs::get_results_2yrs,
        get_results_by_names::get_results_by_names,
        get_results_current_year::{get_results_bests, get_results_current_year},
    },
    meets::get_sessions_for_athletes::get_sessions_for_athletes,
    results::search::search_wrapped,
    users::{
        preferences::{get_preferences, patch_auto_unsave},
        saved_sessions::{
            delete_saved_session, delete_saved_sessions, get_saved_sessions, put_saved_session,
        },
    },
};
use crate::routes::scrapers::{
    SlackConfig, interactions::slack_interactions, slack_commands::slack_commands,
};
use axum::{
    http::{HeaderValue, Method},
    Router,
    routing::{get, patch, post, put},
};
pub use error::AppError;
use routes::{
    clubs::get_all_clubs::get_all_clubs,
    comp_data::{
        get_intl_rankings::get_intl_rankings,
        get_qualifying_totals::get_qualifying_totals,
        get_records::get_records,
        get_standards::get_standards,
        get_wso_list::get_wso_list,
        get_wso_records::{get_wso_age_groups, get_wso_records},
    },
    health::health,
    meets::{
        get_all_meets::list_meets_next_3months, get_athletes_by_meet::get_athletes_by_meet,
        get_meet_details::get_meet_details, get_meet_package::get_meet_package,
        get_meet_schedule::get_meet_schedule,
    },
};
use sqlx::PgPool;
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub slack: SlackConfig,
}

pub fn load_env() {
    let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env");
    if env_path.exists() {
        dotenvy::from_path(env_path).expect("failed to load meetcal-backend/.env");
    }
}

pub async fn run(listener: TcpListener, db: PgPool) {
    let cors = CorsLayer::new()
        .allow_origin([
            "https://meetcal.app".parse::<HeaderValue>().unwrap(),
            "https://www.meetcal.app".parse::<HeaderValue>().unwrap(),
            "http://localhost:3000".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:3000".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods([Method::GET]);

    let app = Router::new()
        .route("/health", get(health))
        .route("/clubs", get(get_all_clubs))
        .route("/clubs/athletes", get(get_athletes_by_club))
        .route("/clubs/meet-stats", get(get_meet_stats))
        .route("/data/records", get(get_records))
        .route("/data/wso", get(get_wso_list))
        .route("/data/wso/", get(get_wso_list))
        .route("/data/wso/age-groups", get(get_wso_age_groups))
        .route("/data/wso/records", get(get_wso_records))
        .route("/data/standards", get(get_standards))
        .route("/data/qualifying-totals", get(get_qualifying_totals))
        .route("/data/intl-rankings", get(get_intl_rankings))
        .route("/data/nat-rankings", get(get_national_rankings))
        .route("/data/adaptive", get(get_adaptive_records))
        .route("/meets", get(list_meets_next_3months))
        .route("/meets/details", get(get_meet_details))
        .route("/meets/package", get(get_meet_package))
        .route("/meets/schedule", get(get_meet_schedule))
        .route("/meets/athletes", get(get_athletes_by_meet))
        .route("/meets/athletes-sessions", get(get_sessions_for_athletes))
        .route("/lifting-results", get(get_lifting_results))
        .route("/lifting-results/by-names", get(get_results_by_names))
        .route("/lifting-results/recent", get(get_results_2yrs))
        .route("/lifting-results/year", get(get_results_current_year))
        .route("/lifting-results/bests", get(get_results_bests))
        .route("/search", get(search_wrapped))
        .route(
            "/users/me/saved-sessions",
            get(get_saved_sessions).delete(delete_saved_sessions),
        )
        .route(
            "/users/me/saved-sessions/{session_id}",
            put(put_saved_session).delete(delete_saved_session),
        )
        .route("/users/me/preferences", get(get_preferences))
        .route(
            "/users/me/preferences/auto-unsave",
            patch(patch_auto_unsave),
        )
        .route("/scrapers/slack/commands", post(slack_commands))
        .route("/scrapers/slack/interactions", post(slack_interactions))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(15),
        ))
        .with_state(AppState {
            db,
            slack: SlackConfig::from_env(),
        });

    axum::serve(listener, app).await.unwrap();
}
