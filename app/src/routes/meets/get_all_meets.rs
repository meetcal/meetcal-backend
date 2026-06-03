use crate::{AppError, AppState};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct MeetsList3Months {
    pub names: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
struct MeetName {
    name: String,
}

/// /meets endpoint
///
/// curl 'localhost:3000/meets' | jq .
///
/// This endpoint takes no input and returns a list of meets in the db in the next 3 months sorted
/// by earlist to latest
///
/// "names": [
///   "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
///   "2026 Mountain North WSO Championships",
///   "2026 Ohio WSO Championships"
/// ]
pub async fn list_meets_next_3months(
    State(state): State<AppState>,
) -> Result<Json<MeetsList3Months>, AppError> {
    let rows = sqlx::query_as::<_, MeetName>(
        r#"
        SELECT name 
        FROM meets 
        WHERE status != 'completed' 
            AND start_date >= CURRENT_DATE 
            AND start_date <= CURRENT_DATE + INTERVAL '3 months' 
        ORDER BY start_date asc 
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(MeetsList3Months {
        names: rows.into_iter().map(|n| n.name).collect(),
    }))
}
