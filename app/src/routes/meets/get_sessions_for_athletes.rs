use crate::{AppError, AppState};
use axum::extract::State;
use axum::{Json, extract::Query};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionsAthletesParams {
    pub meet: String,
    pub session_number: Option<f64>,
    pub platform: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, FromRow)]
pub struct SessionsAthletes {
    pub member_id: String,
    pub name: String,
    pub age: f64,
    pub club: String,
    pub wso: Option<String>,
    pub gender: String,
    pub weight_class: String,
    pub entry_total: f64,
    pub adaptive: bool,
    pub session_number: Option<f64>,
    pub session_platform: Option<String>,
    pub date: Option<String>,
    pub start_time: Option<String>,
    pub weigh_in_time: Option<String>,
}

/// One projection for all four filter combinations, so the column list and the
/// schedule join condition exist once.
///
/// `$join` is `LEFT JOIN` only for the unfiltered variant, which must still
/// list athletes whose session has no schedule row yet; the filtered variants
/// match on a session/platform that by definition has one. `$filters` is
/// appended to the `WHERE`. Both are literals written here, never caller input,
/// and the expansion is a string literal so the query stays `&'static str`.
macro_rules! sessions_for_athletes_sql {
    ($join:literal, $filters:literal) => {
        concat!(
            r#"
        SELECT
            a.member_id,
            a.name,
            a.age,
            a.club,
            a.wso,
            a.gender,
            a.weight_class,
            a.entry_total,
            a.adaptive,
            a.session_number,
            a.session_platform,
            s.date,
            s.start_time,
            s.weigh_in_time
        FROM athletes a
        "#,
            $join,
            r#" session_schedule s
            ON s.meet = a.meet
            AND s.session_id = a.session_number
            AND s.platform = a.session_platform
        WHERE a.meet = $1
            "#,
            $filters,
            r#"
        "#
        )
    };
}

const BY_SESSION_AND_PLATFORM_SQL: &str = sessions_for_athletes_sql!(
    "JOIN",
    "AND a.session_number = $2\n            AND a.session_platform = $3"
);
const BY_SESSION_SQL: &str = sessions_for_athletes_sql!("JOIN", "AND a.session_number = $2");
const BY_PLATFORM_SQL: &str = sessions_for_athletes_sql!("JOIN", "AND a.session_platform = $2");
const ALL_SESSIONS_SQL: &str = sessions_for_athletes_sql!("LEFT JOIN", "");

/// /meets/athletes-sessions endpoint
///
/// curl 'https://api.meetcal.app/meets/athletes-sessions?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness' | jq .
///
/// This endpoint takes meet name and returns athletes and their session rows.
/// Optional session_number and platform filters return one session/platform.
///
/// [
///     {
///       "member_id": "12345",
///       "name": "Jane Doe",
///       "age": 28,
///       "club": "ABC Weightlifting",
///       "wso": "Mountain South",
///       "gender": "Women",
///       "weight_class": "71kg",
///       "entry_total": 180,
///       "adaptive": false,
///       "session_number": 3,
///       "session_platform": "Red",
///       "date": "2025-05-31",
///       "start_time": "10:00",
///       "weigh_in_time": "08:00",
///     }
/// ]
pub async fn get_sessions_for_athletes(
    State(state): State<AppState>,
    Query(params): Query<SessionsAthletesParams>,
) -> Result<Json<Vec<SessionsAthletes>>, AppError> {
    crate::common::query::require_non_empty("meet", &params.meet)?;
    let rows: Vec<SessionsAthletes> = match (params.session_number, params.platform) {
        (Some(session_number), Some(platform)) => {
            sqlx::query_as(BY_SESSION_AND_PLATFORM_SQL)
                .bind(&params.meet)
                .bind(session_number)
                .bind(platform)
                .fetch_all(&state.db)
                .await?
        }
        (Some(session_number), None) => {
            sqlx::query_as(BY_SESSION_SQL)
                .bind(&params.meet)
                .bind(session_number)
                .fetch_all(&state.db)
                .await?
        }
        (None, Some(platform)) => {
            sqlx::query_as(BY_PLATFORM_SQL)
                .bind(&params.meet)
                .bind(platform)
                .fetch_all(&state.db)
                .await?
        }
        (None, None) => {
            sqlx::query_as(ALL_SESSIONS_SQL)
                .bind(&params.meet)
                .fetch_all(&state.db)
                .await?
        }
    };

    Ok(Json(rows))
}
