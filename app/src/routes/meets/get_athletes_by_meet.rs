use crate::{
    AppError, AppState,
    routes::meets::types::{Athlete, MeetsParams},
};
use axum::{
    Json,
    extract::{Query, State},
};

/// /meets/athletes/{name} endpoint
///
/// curl 'localhost:3000/meets/athletes?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness' | jq .
///
/// This endpoint takes the name of the meet exactly as it shows in BARS and returns the athletes in
/// the meet
///
/// Get meet names as they are listed by copying exact case-sensitive names from BARS
///
/// [
///  {
///    "adaptive": false,
///    "age": 27.0,
///    "club": "Vardanian Weightlifting",
///    "entry_total": 365.0,
///    "gender": "Male",
///    "meet": "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
///    "name": "Kyle Schulman",
///    "session_number": 45.0,
///    "session_platform": "Red",
///    "weight_class": "+110",
///    "wso": null
///  },
/// ]
pub async fn get_athletes_by_meet(
    State(state): State<AppState>,
    Query(params): Query<MeetsParams>,
) -> Result<Json<Vec<Athlete>>, AppError> {
    let mut rows = sqlx::query_as::<_, Athlete>(
        r#"
        SELECT adaptive, age, club, entry_total, gender, meet, name,
               session_number, session_platform, weight_class, wso
        FROM athletes
        WHERE meet = $1
        "#,
    )
    .bind(params.meet)
    .fetch_all(&state.db)
    .await?;

    rows.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(rows))
}
