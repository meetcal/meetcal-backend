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
            AppError::Convex(err) => {
                eprintln!("upstream service error: {err:#}");
                (
                    StatusCode::BAD_GATEWAY,
                    "Upstream service error".to_string(),
                )
            }
            AppError::Database(err) => {
                eprintln!("database error: {err}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::NotFound => (StatusCode::NOT_FOUND, "Not found".to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()),
            AppError::Validation(message) => (StatusCode::BAD_REQUEST, message),
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
}
