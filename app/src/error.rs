use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    Convex(anyhow::Error),
    NotFound,
    Unauthorized,
    Validation(String),
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Convex(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Convex(err) => (StatusCode::BAD_GATEWAY, err.to_string()),
            AppError::NotFound => (StatusCode::NOT_FOUND, "Not found".to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()),
            AppError::Validation(message) => (StatusCode::BAD_REQUEST, message),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
