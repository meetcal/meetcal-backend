use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    Convex(anyhow::Error),
    Database(sqlx::Error),
    NotFound,
    Unauthorized,
    Validation(String),
    /// Upstream dependency failure (e.g. JWKS unreachable) — 503, distinct from a
    /// token rejection (401) so clients don't read it as "signed out".
    Unavailable(String),
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Convex(err)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Database(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Convex(err) => (StatusCode::BAD_GATEWAY, err.to_string()),
            AppError::Database(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            AppError::NotFound => (StatusCode::NOT_FOUND, "Not found".to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()),
            AppError::Validation(message) => (StatusCode::BAD_REQUEST, message),
            AppError::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
