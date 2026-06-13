use crate::{AppError, AppState, routes::meets::types::Meets};
use axum::Json;
use axum::extract::State;

/// /meets endpoint
///
/// curl 'https://api.meetcal.app/meets' | jq .
///
/// This endpoint takes no input and returns a list of meets in the db in the next 3 months sorted
/// by earlist to latest
///
/// [
///   {
///     "end_date": "2026-08-16",
///     "name": "2026 Ohio WSO Championships",
///     "start_date": "2026-08-15",
///     "time_zone": "America/New_York",
///     "venue_city": "Columbus",
///     "venue_name": "2026 Ohio WSO Championships",
///     "venue_state": "OH",
///     "venue_street": "400 North High Street",
///     "venue_zip": "43215"
///     "status": "completed"
///   }
/// ]
pub async fn list_meets_next_3months(
    State(state): State<AppState>,
) -> Result<Json<Vec<Meets>>, AppError> {
    let rows = sqlx::query_as::<_, Meets>(
        r#"
        SELECT name, start_date::text as start_date, end_date::text as end_date, time_zone, venue_city, venue_state, venue_name, venue_street, venue_zip, federation, status FROM meets 
        WHERE status != 'completed' 
            AND start_date >= CURRENT_DATE 
            AND start_date <= CURRENT_DATE + INTERVAL '3 months' 
        ORDER BY start_date asc 
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows))
}
