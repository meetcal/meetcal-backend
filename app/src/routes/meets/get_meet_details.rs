use crate::{
    AppError, AppState,
    routes::meets::types::{Meets, MeetsParams},
};
use axum::{
    Json,
    extract::{Query, State},
};

/// /meets/details endpoint
///
/// curl 'https://api.meetcal.app/meets/details?meet=2026%20Ohio%20WSO%20Championships' | jq .
///
/// This endpoint takes the name of the meet exactly as it shows in BARS and returns the details of
/// the meet
///
/// Get meet names as they are listed by copying exact case-sensitive names from BARS
///
/// {
///   "end_date": "2026-08-16",
///   "name": "2026 Ohio WSO Championships",
///   "start_date": "2026-08-15",
///   "time_zone": "America/New_York",
///   "venue_city": "Columbus",
///   "venue_name": "2026 Ohio WSO Championships",
///   "venue_state": "OH",
///   "venue_street": "400 North High Street",
///   "venue_zip": "43215"
///   "status": "completed"
/// }
pub async fn get_meet_details(
    State(state): State<AppState>,
    Query(params): Query<MeetsParams>,
) -> Result<Json<Meets>, AppError> {
    let rows = sqlx::query_as::<_, Meets>(
        r#"
        SELECT name, start_date::text as start_date, end_date::text as end_date, time_zone, venue_city, venue_state, venue_name, venue_street, venue_zip, federation, status
        FROM meets
        WHERE name = $1
        "#,
    )
    .bind(params.meet)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(rows))
}
