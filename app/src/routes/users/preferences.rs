use crate::{
    AppError, AppState,
    routes::users::auth::{set_request_user, user_id_from_headers},
};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
pub struct UserPreferencesResponse {
    pub auto_unsave_started_sessions: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AutoUnsaveRequest {
    pub enabled: bool,
}

/// /users/me/preferences endpoint
///
/// curl 'https://api.meetcal.app/users/me/preferences' \
///   -H 'Authorization: Bearer <clerk-jwt>' | jq .
///
/// This endpoint returns the authenticated user's profile preferences.
///
/// {
///   "auto_unsave_started_sessions": false
/// }
///
pub async fn get_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UserPreferencesResponse>, AppError> {
    let user_id = user_id_from_headers(&headers)?;
    let mut tx = state.db.begin().await?;
    set_request_user(&mut tx, &user_id).await?;

    let auto_unsave_started_sessions = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT auto_unsave_started_sessions
        FROM user_preferences
        WHERE user_id = $1
        "#,
    )
    .bind(&user_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(false);

    tx.commit().await?;

    Ok(Json(UserPreferencesResponse {
        auto_unsave_started_sessions,
    }))
}

/// /users/me/preferences/auto-unsave endpoint
///
/// curl -X PATCH 'https://api.meetcal.app/users/me/preferences/auto-unsave' \
///   -H 'Authorization: Bearer <clerk-jwt>' \
///   -H 'Content-Type: application/json' \
///   -d '{"enabled":true}' | jq .
///
/// This endpoint updates whether started saved sessions should be removed automatically.
///
/// {
///   "auto_unsave_started_sessions": true
/// }
///
pub async fn patch_auto_unsave(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AutoUnsaveRequest>,
) -> Result<Json<UserPreferencesResponse>, AppError> {
    let user_id = user_id_from_headers(&headers)?;
    let updated_at = now_millis()?;
    let convex_id = format!("user_preferences:{user_id}");
    let mut tx = state.db.begin().await?;
    set_request_user(&mut tx, &user_id).await?;

    let auto_unsave_started_sessions = sqlx::query_scalar::<_, bool>(
        r#"
        INSERT INTO user_preferences (
            convex_id,
            user_id,
            auto_unsave_started_sessions,
            updated_at
        )
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (user_id) DO UPDATE SET
            auto_unsave_started_sessions = EXCLUDED.auto_unsave_started_sessions,
            updated_at = EXCLUDED.updated_at
        RETURNING auto_unsave_started_sessions
        "#,
    )
    .bind(convex_id)
    .bind(&user_id)
    .bind(body.enabled)
    .bind(updated_at)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(UserPreferencesResponse {
        auto_unsave_started_sessions,
    }))
}

fn now_millis() -> Result<i64, AppError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::Validation("system clock is before UNIX epoch".to_string()))?;

    Ok(duration.as_millis() as i64)
}
