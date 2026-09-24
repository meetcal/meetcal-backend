use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
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
    /// The server is at a work limit it enforces (e.g. concurrent package
    /// builds). Answered `503`; clients treat it like any 5xx and retry.
    Busy,
    /// The request body exceeded the route's `DefaultBodyLimit`. Answered
    /// `413` with the standard error body (see `payload_too_large_as_json`).
    PayloadTooLarge,
    /// The client spent its rate-limit bucket (`common::rate_limit`).
    /// Answered `429` with `Retry-After` in whole seconds, the time until the
    /// bucket holds this request's cost again.
    RateLimited {
        retry_after_secs: u64,
    },
    /// The server-wide in-flight cap is reached (`common::load_shed`).
    /// Answered `503` with `Retry-After: 1`: the cap frees as fast as requests
    /// finish, so a prompt retry usually succeeds.
    Overloaded,
}

/// `Retry-After` on an [`AppError::Overloaded`] answer.
pub const OVERLOADED_RETRY_AFTER_SECS: u64 = 1;

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
        let mut retry_after = None;
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
            AppError::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy".to_string()),
            AppError::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body too large".to_string(),
            ),
            AppError::RateLimited { retry_after_secs } => {
                retry_after = Some(retry_after_secs);
                (StatusCode::TOO_MANY_REQUESTS, "rate limited".to_string())
            }
            AppError::Overloaded => {
                retry_after = Some(OVERLOADED_RETRY_AFTER_SECS);
                (StatusCode::SERVICE_UNAVAILABLE, "overloaded".to_string())
            }
        };

        let mut response = (status, Json(json!({ "error": message }))).into_response();
        if let Some(secs) = retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        response
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
    async fn busy_is_a_503_with_the_error_body_shape() {
        let response = AppError::Busy.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), br#"{"error":"busy"}"#);
    }

    #[tokio::test]
    async fn payload_too_large_is_a_413_with_the_error_body_shape() {
        let response = AppError::PayloadTooLarge.into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), br#"{"error":"request body too large"}"#);
    }

    #[tokio::test]
    async fn rate_limited_is_a_429_with_retry_after_and_the_error_body_shape() {
        let response = AppError::RateLimited {
            retry_after_secs: 7,
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "7");
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), br#"{"error":"rate limited"}"#);
    }

    #[tokio::test]
    async fn overloaded_is_a_503_with_retry_after_one_second() {
        let response = AppError::Overloaded.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "1");
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), br#"{"error":"overloaded"}"#);
    }

    #[tokio::test]
    async fn busy_sends_no_retry_after() {
        let response = AppError::Busy.into_response();
        assert!(response.headers().get(header::RETRY_AFTER).is_none());
    }

    #[tokio::test]
    async fn missing_rows_are_not_found() {
        let response = AppError::from(sqlx::Error::RowNotFound).into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), br#"{"error":"Not found"}"#);
    }
}
