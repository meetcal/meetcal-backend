use crate::AppError;
use axum::http::HeaderMap;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use sqlx::{Postgres, Transaction};

#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: String,
}

pub fn user_id_from_headers(headers: &HeaderMap) -> Result<String, AppError> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;

    let payload = token.split('.').nth(1).ok_or(AppError::Unauthorized)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| AppError::Unauthorized)?;
    let claims: JwtClaims = serde_json::from_slice(&decoded).map_err(|_| AppError::Unauthorized)?;

    if claims.sub.trim().is_empty() {
        return Err(AppError::Unauthorized);
    }

    Ok(claims.sub)
}

pub async fn set_request_user(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
) -> Result<(), AppError> {
    sqlx::query("SELECT set_config('request.jwt.claim.sub', $1, true)")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;

    Ok(())
}
