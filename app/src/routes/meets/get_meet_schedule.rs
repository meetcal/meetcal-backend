use crate::{
    AppError, AppState,
    routes::meets::types::{MeetSchedule, MeetsParams},
};
use axum::{
    Json,
    extract::{Query, State},
};

/// /meets/schedule/{name} endpoint
///
/// curl 'https://api.meetcal.app/meets/schedule?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness' | jq .
///
/// This endpoint takes the name of the meet exactly as it shows in BARS and returns the schedule of
/// the meet
///
/// Get meet names as they are listed by copying exact case-sensitive names from BARS
///
/// [
///  {
///    "date": "2026-06-20",
///    "meet": "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
///    "platform": "Red",
///    "session_id": 1.0,
///    "start_time": "08:00:00",
///    "weigh_in_time": "06:00:00",
///    "weight_class": "40kg B"
///  },
/// ]
pub async fn get_meet_schedule(
    State(state): State<AppState>,
    Query(params): Query<MeetsParams>,
) -> Result<Json<Vec<MeetSchedule>>, AppError> {
    let mut rows = sqlx::query_as::<_, MeetSchedule>(
        r#"
        SELECT date, meet, platform, session_id, start_time, weigh_in_time, weight_class
        FROM session_schedule
        WHERE meet = $1
        "#,
    )
    .bind(params.meet)
    .fetch_all(&state.db)
    .await?;

    rows.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| {
                a.session_id
                    .partial_cmp(&b.session_id)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.platform.cmp(&b.platform))
    });

    Ok(Json(rows))
}
