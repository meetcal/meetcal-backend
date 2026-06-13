use crate::{
    AppError, AppState,
    routes::users::auth::{set_request_user, user_id_from_headers},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
pub struct DeleteSavedSessionsParams {
    pub meet: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SavedSessionRequest {
    pub meet: String,
    pub session_number: f64,
    pub platform: String,
    pub weight_class: Option<String>,
    pub start_time: Option<String>,
    pub date: Option<String>,
    pub notes: Option<String>,
    pub athlete_names: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct SavedSession {
    pub session_id: String,
    pub meet: String,
    pub session_number: f64,
    pub platform: String,
    pub weight_class: Option<String>,
    pub start_time: Option<String>,
    pub date: Option<String>,
    pub notes: Option<String>,
    pub athlete_names: Vec<String>,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedSessionsResponse {
    pub sessions: Vec<SavedSession>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveSessionResponse {
    pub session_id: String,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteSavedSessionResponse {
    pub deleted: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteSavedSessionsResponse {
    pub deleted_count: i64,
}

/// /users/me/saved-sessions endpoint
///
/// curl 'https://api.meetcal.app/users/me/saved-sessions' \
///   -H 'Authorization: Bearer <clerk-jwt>' | jq .
///
/// This endpoint returns the authenticated user's saved platform sessions for cross-device sync.
///
/// {
///   "sessions": [
///     {
///       "session_id": "2025-Nationals-3-Red",
///       "meet": "2025 Nationals",
///       "session_number": 3.0,
///       "platform": "Red",
///       "weight_class": "73kg",
///       "start_time": "10:00 AM",
///       "date": "2025-05-31",
///       "notes": "optional",
///       "athlete_names": ["Jane Doe"],
///       "updated_at": 1717171717000
///     }
///   ]
/// }
///
pub async fn get_saved_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SavedSessionsResponse>, AppError> {
    let user_id = user_id_from_headers(&headers)?;
    let mut tx = state.db.begin().await?;
    set_request_user(&mut tx, &user_id).await?;

    let sessions = sqlx::query_as::<_, SavedSession>(
        r#"
        SELECT
            session_id,
            meet,
            session_number,
            platform,
            weight_class,
            start_time,
            date,
            notes,
            COALESCE(athlete_names, ARRAY[]::text[]) AS athlete_names,
            updated_at
        FROM saved_sessions
        WHERE user_id = $1
        ORDER BY meet, date, session_number, platform
        "#,
    )
    .bind(&user_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(SavedSessionsResponse { sessions }))
}

/// /users/me/saved-sessions/:session_id endpoint
///
/// curl -X PUT 'https://api.meetcal.app/users/me/saved-sessions/2025-Nationals-3-Red' \
///   -H 'Authorization: Bearer <clerk-jwt>' \
///   -H 'Content-Type: application/json' \
///   -d '{"meet":"2025 Nationals","session_number":3,"platform":"Red","weight_class":"73kg","start_time":"10:00 AM","date":"2025-05-31","notes":"optional","athlete_names":["Jane Doe"]}' | jq .
///
/// This endpoint upserts one saved platform session for the authenticated user.
///
/// {
///   "session_id": "2025-Nationals-3-Red",
///   "updated_at": 1717171717000
/// }
///
pub async fn put_saved_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(body): Json<SavedSessionRequest>,
) -> Result<Json<SaveSessionResponse>, AppError> {
    if session_id.trim().is_empty() {
        return Err(AppError::Validation("session_id is required".to_string()));
    }

    let user_id = user_id_from_headers(&headers)?;
    let updated_at = now_millis()?;
    let athlete_names = body.athlete_names.unwrap_or_default();
    let convex_id = format!("saved_session:{user_id}:{session_id}");

    let mut tx = state.db.begin().await?;
    set_request_user(&mut tx, &user_id).await?;

    let row: (String, i64) = sqlx::query_as(
        r#"
        INSERT INTO saved_sessions (
            convex_id,
            session_id,
            user_id,
            meet,
            session_number,
            platform,
            weight_class,
            start_time,
            notes,
            athlete_names,
            date,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT (session_id, user_id) DO UPDATE SET
            meet = EXCLUDED.meet,
            session_number = EXCLUDED.session_number,
            platform = EXCLUDED.platform,
            weight_class = EXCLUDED.weight_class,
            start_time = EXCLUDED.start_time,
            notes = EXCLUDED.notes,
            athlete_names = EXCLUDED.athlete_names,
            date = EXCLUDED.date,
            updated_at = EXCLUDED.updated_at
        RETURNING session_id, updated_at
        "#,
    )
    .bind(convex_id)
    .bind(&session_id)
    .bind(&user_id)
    .bind(body.meet)
    .bind(body.session_number)
    .bind(body.platform)
    .bind(body.weight_class)
    .bind(body.start_time)
    .bind(body.notes)
    .bind(athlete_names)
    .bind(body.date)
    .bind(updated_at)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(SaveSessionResponse {
        session_id: row.0,
        updated_at: row.1,
    }))
}

/// /users/me/saved-sessions/:session_id endpoint
///
/// curl -X DELETE 'https://api.meetcal.app/users/me/saved-sessions/2025-Nationals-3-Red' \
///   -H 'Authorization: Bearer <clerk-jwt>' | jq .
///
/// This endpoint deletes one saved platform session for the authenticated user.
///
/// {
///   "deleted": true
/// }
///
pub async fn delete_saved_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<DeleteSavedSessionResponse>, AppError> {
    let user_id = user_id_from_headers(&headers)?;
    let mut tx = state.db.begin().await?;
    set_request_user(&mut tx, &user_id).await?;

    let deleted = sqlx::query_scalar::<_, i64>(
        r#"
        DELETE FROM saved_sessions
        WHERE user_id = $1
            AND session_id = $2
        RETURNING 1::BIGINT
        "#,
    )
    .bind(&user_id)
    .bind(session_id)
    .fetch_optional(&mut *tx)
    .await?
    .is_some();

    tx.commit().await?;

    Ok(Json(DeleteSavedSessionResponse { deleted }))
}

/// /users/me/saved-sessions endpoint
///
/// curl -X DELETE 'https://api.meetcal.app/users/me/saved-sessions?meet=2025%20Nationals' \
///   -H 'Authorization: Bearer <clerk-jwt>' | jq .
///
/// This endpoint deletes all saved sessions for the authenticated user, optionally scoped to one meet.
///
/// {
///   "deleted_count": 4
/// }
///
pub async fn delete_saved_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<DeleteSavedSessionsParams>,
) -> Result<Json<DeleteSavedSessionsResponse>, AppError> {
    let user_id = user_id_from_headers(&headers)?;
    let mut tx = state.db.begin().await?;
    set_request_user(&mut tx, &user_id).await?;

    let deleted_count = if let Some(meet) = params.meet {
        sqlx::query_scalar::<_, i64>(
            r#"
            WITH deleted AS (
                DELETE FROM saved_sessions
                WHERE user_id = $1
                    AND meet = $2
                RETURNING 1
            )
            SELECT COUNT(*)::BIGINT FROM deleted
            "#,
        )
        .bind(&user_id)
        .bind(meet)
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            r#"
            WITH deleted AS (
                DELETE FROM saved_sessions
                WHERE user_id = $1
                RETURNING 1
            )
            SELECT COUNT(*)::BIGINT FROM deleted
            "#,
        )
        .bind(&user_id)
        .fetch_one(&mut *tx)
        .await?
    };

    tx.commit().await?;

    Ok(Json(DeleteSavedSessionsResponse { deleted_count }))
}

fn now_millis() -> Result<i64, AppError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::Validation("system clock is before UNIX epoch".to_string()))?;

    Ok(duration.as_millis() as i64)
}
