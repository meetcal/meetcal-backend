use crate::{
    AppError, AppState,
    common::time::now_millis,
    routes::users::auth::{set_request_user, user_id_from_headers},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;

/// Longest `session_id` path segment. Ids are `<meet>-<session>-<platform>`
/// built by the app, so a real one is well under this.
pub const MAX_SAVED_SESSION_ID_LEN: usize = 256;
/// Longest `meet` name. Sanctioned meet names run to ~80 characters.
pub const MAX_SAVED_SESSION_MEET_LEN: usize = 256;
/// Longest `platform`, `weight_class`, `start_time`, or `date` value; each is
/// a short token such as `Red`, `+110`, `08:00:00`, `2026-06-20`.
pub const MAX_SAVED_SESSION_FIELD_LEN: usize = 64;
/// Longest free-text `notes` value.
pub const MAX_SAVED_SESSION_NOTES_LEN: usize = 2000;
/// Longest single `athlete_names` entry (the list itself is bounded by
/// [`crate::common::query::MAX_SAVED_SESSION_ATHLETE_NAMES`]).
pub const MAX_SAVED_SESSION_ATHLETE_NAME_LEN: usize = 128;
/// Ceiling on saved sessions per user. A meet has tens of sessions and a user
/// follows a few meets at a time; the cap keeps one account from growing the
/// table without bound.
pub const MAX_SAVED_SESSIONS_PER_USER: usize = 500;

/// Errors from a saved-session write. A cap overrun is reported with the cap
/// itself so the client can show it (`{"error": "...", "max": N}`); every
/// other failure is an ordinary [`AppError`].
#[derive(Debug)]
pub enum SaveSessionError {
    App(AppError),
    OverLimit { what: &'static str, max: usize },
}

impl From<AppError> for SaveSessionError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl From<sqlx::Error> for SaveSessionError {
    fn from(error: sqlx::Error) -> Self {
        Self::App(error.into())
    }
}

impl IntoResponse for SaveSessionError {
    fn into_response(self) -> Response {
        match self {
            Self::App(error) => error.into_response(),
            Self::OverLimit { what, max } => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": what, "max": max })),
            )
                .into_response(),
        }
    }
}

/// Reject a value longer than `max` characters with the cap in the body.
fn require_max_len(
    what: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<(), SaveSessionError> {
    match value {
        Some(value) if value.chars().count() > max => {
            Err(SaveSessionError::OverLimit { what, max })
        }
        _ => Ok(()),
    }
}

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
    let user_id = user_id_from_headers(&headers, state.auth.as_deref()).await?;
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
) -> Result<Json<SaveSessionResponse>, SaveSessionError> {
    if session_id.trim().is_empty() {
        return Err(AppError::Validation("session_id is required".to_string()).into());
    }
    crate::common::query::require_non_empty("meet", &body.meet)?;
    crate::common::query::require_non_empty("platform", &body.platform)?;
    require_max_len(
        "session_id too long",
        Some(&session_id),
        MAX_SAVED_SESSION_ID_LEN,
    )?;
    require_max_len(
        "meet too long",
        Some(&body.meet),
        MAX_SAVED_SESSION_MEET_LEN,
    )?;
    require_max_len(
        "platform too long",
        Some(&body.platform),
        MAX_SAVED_SESSION_FIELD_LEN,
    )?;
    require_max_len(
        "weight_class too long",
        body.weight_class.as_deref(),
        MAX_SAVED_SESSION_FIELD_LEN,
    )?;
    require_max_len(
        "start_time too long",
        body.start_time.as_deref(),
        MAX_SAVED_SESSION_FIELD_LEN,
    )?;
    require_max_len(
        "date too long",
        body.date.as_deref(),
        MAX_SAVED_SESSION_FIELD_LEN,
    )?;
    require_max_len(
        "notes too long",
        body.notes.as_deref(),
        MAX_SAVED_SESSION_NOTES_LEN,
    )?;

    let athlete_names = body.athlete_names.unwrap_or_default();
    if athlete_names.len() > crate::common::query::MAX_SAVED_SESSION_ATHLETE_NAMES {
        return Err(SaveSessionError::OverLimit {
            what: "too many athlete_names",
            max: crate::common::query::MAX_SAVED_SESSION_ATHLETE_NAMES,
        });
    }
    for name in &athlete_names {
        require_max_len(
            "athlete_names entry too long",
            Some(name),
            MAX_SAVED_SESSION_ATHLETE_NAME_LEN,
        )?;
    }

    let user_id = user_id_from_headers(&headers, state.auth.as_deref()).await?;
    let updated_at = now_millis()?;
    let row_id = format!("saved_session:{user_id}:{session_id}");

    let mut tx = state.db.begin().await?;
    set_request_user(&mut tx, &user_id).await?;

    // Bound rows per user. Updating an existing session never counts against
    // the cap, so a full account can still edit what it has.
    let existing_others = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::BIGINT
        FROM saved_sessions
        WHERE user_id = $1
            AND session_id <> $2
        "#,
    )
    .bind(&user_id)
    .bind(&session_id)
    .fetch_one(&mut *tx)
    .await?;
    if usize::try_from(existing_others).unwrap_or(usize::MAX) >= MAX_SAVED_SESSIONS_PER_USER {
        return Err(SaveSessionError::OverLimit {
            what: "too many saved sessions",
            max: MAX_SAVED_SESSIONS_PER_USER,
        });
    }

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
    .bind(row_id)
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
    let user_id = user_id_from_headers(&headers, state.auth.as_deref()).await?;
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
    let user_id = user_id_from_headers(&headers, state.auth.as_deref()).await?;
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
