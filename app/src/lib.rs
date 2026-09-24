pub mod common;
pub mod configuration;
pub mod error;
pub mod routes;

use crate::routes::scrapers::{
    SlackConfig, interactions::slack_interactions, slack_commands::slack_commands,
};
use crate::routes::{
    clubs::{get_athletes_by_club::get_athletes_by_club, get_meet_stats::get_meet_stats},
    comp_data::{
        get_adaptive_records::get_adaptive_records,
        get_national_ranking_by_year::get_national_rankings_by_year,
        get_national_rankings::get_national_rankings,
    },
    lifting_results::{
        get_lifting_results::get_lifting_results,
        get_results_2yrs::{get_results_2yrs, post_results_2yrs},
        get_results_by_names::{get_results_by_names, post_results_by_names},
        get_results_current_year::{
            get_results_bests, get_results_current_year, post_results_bests,
        },
    },
    meets::get_sessions_for_athletes::get_sessions_for_athletes,
    results::search::search_wrapped,
    users::{
        preferences::{get_preferences, patch_auto_unsave},
        saved_sessions::{
            delete_saved_session, delete_saved_sessions, get_saved_sessions, put_saved_session,
        },
    },
    wsos::get_athletes_by_wso::get_athletes_by_wso,
};
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
};
use common::client::CLIENT_VERSION_HEADER;
use common::query::NAME_LIST_BODY_LIMIT;
pub use error::AppError;
use routes::users::USER_WRITE_BODY_LIMIT;
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
        get_all_meets::{list_completed_meets, list_meets_next_3months},
        get_athletes_by_meet::get_athletes_by_meet,
        get_meet_details::get_meet_details,
        get_meet_package::get_meet_package,
        get_meet_schedule::get_meet_schedule,
    },
};
use sqlx::PgPool;
use std::path::PathBuf;
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

/// Wall-clock ceiling on one HTTP request. A read that outruns it answers
/// `408 {"error":"timeout"}` instead of holding a pool connection for the
/// client's lifetime. `AGENTS.md` pins this at 15s.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Drops the handler once `limit` elapses and answers [`AppError::Timeout`],
/// so a timeout has the same JSON error body as every other failure. Dropping
/// the handler future cancels its in-flight query and returns the connection
/// to the pool.
async fn with_timeout(limit: Duration, request: Request, next: Next) -> Response {
    match tokio::time::timeout(limit, next.run(request)).await {
        Ok(response) => response,
        Err(_elapsed) => {
            tracing::warn!(limit_ms = limit.as_millis() as u64, "request timed out");
            AppError::Timeout.into_response()
        }
    }
}

async fn request_timeout(request: Request, next: Next) -> Response {
    with_timeout(REQUEST_TIMEOUT, request, next).await
}

/// Request-body ceiling for every route without a tighter one. The only other
/// bodies the API reads are Slack's form-encoded slash commands and
/// interaction payloads (the message blocks plus action state), which are tens
/// of kilobytes at most. 1 MiB keeps generous headroom for those while halving
/// axum's implicit 2 MB default. The name-list and `/users/me/*` write routes
/// use [`NAME_LIST_BODY_LIMIT`] and [`USER_WRITE_BODY_LIMIT`] instead.
pub const DEFAULT_BODY_LIMIT: usize = 1024 * 1024;

/// Gives a body-limit rejection the same `{"error": ...}` JSON body as every
/// other failure. Axum's extractors (`Json`, `Bytes`, `Form`) answer an
/// oversized body with `413` and a plain-text message; this rewrites only
/// that plain-text form, so a handler's own JSON `413` would pass through.
async fn payload_too_large_as_json(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes().starts_with(b"application/json"));
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE && !is_json {
        return AppError::PayloadTooLarge.into_response();
    }
    response
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub slack: SlackConfig,
    pub auth: Option<Arc<routes::users::auth::AuthVerifier>>,
}

pub fn load_env() {
    let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env");
    if env_path.exists() {
        dotenvy::from_path(env_path).expect("failed to load meetcal-backend/.env");
    }
}

pub async fn run(listener: TcpListener, db: PgPool) {
    let auth = routes::users::auth::AuthVerifier::from_env()
        .unwrap_or_else(|error| panic!("invalid Clerk authentication configuration: {error}"));
    if auth.is_none() {
        tracing::warn!(
            "Clerk authentication is not configured; protected user routes will reject all requests"
        );
    }
    run_with_auth(listener, db, auth).await;
}

pub async fn run_with_auth(
    listener: TcpListener,
    db: PgPool,
    auth: Option<Arc<routes::users::auth::AuthVerifier>>,
) {
    let cors = CorsLayer::new()
        .allow_origin([
            "https://meetcal.app".parse::<HeaderValue>().unwrap(),
            "https://www.meetcal.app".parse::<HeaderValue>().unwrap(),
            "http://localhost:3000".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:3000".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::IF_NONE_MATCH,
            HeaderName::from_static(CLIENT_VERSION_HEADER),
        ])
        .expose_headers([header::ETAG]);

    // One span per request carrying method, path, and the declared app
    // version (which decides strict-vs-legacy validation), closed with the
    // status and latency at INFO. Failures (5xx) log at ERROR by default.
    // The path only: query strings carry athlete names (`?names=`, `?name=`,
    // `?query=`), which stay out of the logs.
    let trace = TraceLayer::new_for_http()
        .make_span_with(|request: &Request| {
            let client = request
                .headers()
                .get(CLIENT_VERSION_HEADER)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("-");
            tracing::info_span!(
                "request",
                method = %request.method(),
                path = %request.uri().path(),
                client = client
            )
        })
        .on_response(DefaultOnResponse::new().level(Level::INFO));

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
        .route("/wsos/athletes", get(get_athletes_by_wso))
        .route("/data/standards", get(get_standards))
        .route("/data/qualifying-totals", get(get_qualifying_totals))
        .route("/data/intl-rankings", get(get_intl_rankings))
        .route("/data/nat-rankings", get(get_national_rankings))
        .route(
            "/data/nat-rankings-year",
            get(get_national_rankings_by_year),
        )
        .route("/data/adaptive", get(get_adaptive_records))
        .route("/meets", get(list_meets_next_3months))
        .route("/meets/completed", get(list_completed_meets))
        .route("/meets/details", get(get_meet_details))
        .route("/meets/package", get(get_meet_package))
        .route("/meets/schedule", get(get_meet_schedule))
        .route("/meets/athletes", get(get_athletes_by_meet))
        .route("/meets/athletes-sessions", get(get_sessions_for_athletes))
        .route("/lifting-results", get(get_lifting_results))
        .route(
            "/lifting-results/by-names",
            get(get_results_by_names)
                .post(post_results_by_names)
                .layer(DefaultBodyLimit::max(NAME_LIST_BODY_LIMIT)),
        )
        .route(
            "/lifting-results/recent",
            get(get_results_2yrs)
                .post(post_results_2yrs)
                .layer(DefaultBodyLimit::max(NAME_LIST_BODY_LIMIT)),
        )
        .route("/lifting-results/year", get(get_results_current_year))
        .route(
            "/lifting-results/bests",
            get(get_results_bests)
                .post(post_results_bests)
                .layer(DefaultBodyLimit::max(NAME_LIST_BODY_LIMIT)),
        )
        .route("/search", get(search_wrapped))
        .route(
            "/users/me/saved-sessions",
            get(get_saved_sessions)
                .delete(delete_saved_sessions)
                .layer(DefaultBodyLimit::max(USER_WRITE_BODY_LIMIT)),
        )
        .route(
            "/users/me/saved-sessions/{session_id}",
            put(put_saved_session)
                .delete(delete_saved_session)
                .layer(DefaultBodyLimit::max(USER_WRITE_BODY_LIMIT)),
        )
        .route(
            "/users/me/preferences",
            get(get_preferences).layer(DefaultBodyLimit::max(USER_WRITE_BODY_LIMIT)),
        )
        .route(
            "/users/me/preferences/auto-unsave",
            patch(patch_auto_unsave).layer(DefaultBodyLimit::max(USER_WRITE_BODY_LIMIT)),
        )
        .route("/scrapers/slack/commands", post(slack_commands))
        .route("/scrapers/slack/interactions", post(slack_interactions))
        // Route-level limits above are inner layers, so they override this
        // default for their routes.
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(middleware::from_fn(payload_too_large_as_json))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(middleware::from_fn(request_timeout))
        .layer(trace)
        .with_state(AppState {
            db,
            slack: SlackConfig::from_env(),
            auth,
        });

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

/// Resolves on SIGINT or SIGTERM so `axum::serve` stops accepting connections
/// and lets in-flight requests finish before the process exits. Deploys use
/// `docker rm -f` / systemd stop, which send SIGTERM first.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received; draining in-flight requests");
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use axum::{body::to_bytes, http::StatusCode};
    use tower::ServiceExt;

    async fn slow() -> &'static str {
        tokio::time::sleep(Duration::from_millis(200)).await;
        "done"
    }

    async fn fast() -> &'static str {
        "done"
    }

    #[tokio::test]
    async fn a_slow_handler_answers_408_with_the_json_error_shape() {
        let app = Router::new()
            .route("/slow", get(slow))
            .route("/fast", get(fast))
            .layer(middleware::from_fn(|request, next| {
                with_timeout(Duration::from_millis(20), request, next)
            }));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/slow")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), br#"{"error":"timeout"}"#);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/fast")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
