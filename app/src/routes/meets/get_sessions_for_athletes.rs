use crate::{AppError, AppState};
use axum::extract::State;
use axum::{Json, extract::Query};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionsAthletesParams {
    pub meet: String,
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
    pub session_number: f64,
    pub session_platform: String,
    pub date: String,
    pub start_time: String,
    pub weigh_in_time: String,
}

/// /meets/athletes-sessions endpoint
///
/// curl 'https://api.meetcal.app/meets/athletes-sessions?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness' | jq .
///
/// This endpoint takes meet name and returns athletes and their session rows
///
/// TODO: Keep as a focused route, but the RN app's main prefetch flow should prefer /meets/package
/// so athlete/session joins, schedule rows, and result history are returned together.
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
    let rows: Vec<SessionsAthletes> = sqlx::query_as(
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
        JOIN session_schedule s
            ON s.meet = a.meet
            AND s.session_id = a.session_number
            AND s.platform = a.session_platform
        WHERE a.meet = $1
        "#,
    )
    .bind(params.meet)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows))
}
