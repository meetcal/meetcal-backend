use crate::{
    AppError, AppState, common::query_convex::get_convex_response,
    routes::meets::types::MeetSchedule,
};
use axum::{
    Json,
    extract::{Path, State},
};
use convex::Value;
use std::collections::BTreeMap;

/// /meets/schedule/{name} endpoint
///
/// curl 'localhost:3000/meets/schedule/2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness' | jq .
///
/// This endpoint takes the name of the meet exactly as it shows in BARS and returns the schedule of
/// the meet
///
///[
///  {
///    "date": "2026-06-20",
///    "meet": "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
///    "platform": "Red",
///    "sessionId": 1.0,
///    "startTime": "08:00:00",
///    "weighInTime": "06:00:00",
///    "weightClass": "40kg B"
///  },
///]
pub async fn get_meet_schedule(
    State(state): State<AppState>,
    Path(meet_name): Path<String>,
) -> Result<Json<Vec<MeetSchedule>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert("meet".to_string(), Value::from(meet_name));

    let mut response: Vec<MeetSchedule> =
        get_convex_response(&state.convex, "schedule:getByMeet", args).await?;

    response.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| {
                a.session_id
                    .partial_cmp(&b.session_id)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.platform.cmp(&b.platform))
    });

    Ok(Json(response))
}
