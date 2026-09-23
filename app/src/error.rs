use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    Internal(anyhow::Error),
    Database(sqlx::Error),
    NotFound,
    Unauthorized,
    Validation(String),
    /// The request outran [`crate::REQUEST_TIMEOUT`]. Answered `408` with the
    /// same `{"error": ...}` shape as every other failure so clients parse one
    /// error body.
    Timeout,
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        if matches!(err, sqlx::Error::RowNotFound) {
            AppError::NotFound
        } else {
            AppError::Database(err)
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Internal(err) => {
                tracing::error!(error = format!("{err:#}"), "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::Database(err) => {
                tracing::error!(error = %err, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::NotFound => (StatusCode::NOT_FOUND, "Not found".to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()),
            AppError::Validation(message) => (StatusCode::BAD_REQUEST, message),
            AppError::Timeout => (StatusCode::REQUEST_TIMEOUT, "timeout".to_string()),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn internal_error_details_are_not_returned() {
        let response = AppError::Database(sqlx::Error::Protocol(
            "sensitive database detail".to_string(),
        ))
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
        assert!(!body.contains("sensitive database detail"));
    }

    #[tokio::test]
    async fn timeouts_share_the_error_body_shape() {
        let response = AppError::Timeout.into_response();
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), br#"{"error":"timeout"}"#);
    }

    #[tokio::test]
    async fn missing_rows_are_not_found() {
        let response = AppError::from(sqlx::Error::RowNotFound).into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), br#"{"error":"Not found"}"#);
    }
}
