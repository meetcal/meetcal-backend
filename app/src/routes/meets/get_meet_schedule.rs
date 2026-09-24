use crate::{
    AppError, AppState,
    common::{client::ClientVersion, http_cache::cacheable_json},
    routes::meets::types::{MeetSchedule, MeetsParams},
};
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::Response,
};

/// /meets/schedule/{name} endpoint
///
/// curl 'https://api.meetcal.app/meets/schedule?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness' | jq .
///
/// This endpoint takes the name of the meet exactly as it shows in BARS and returns the schedule of
/// the meet. The body carries a strong `ETag` and `Cache-Control: no-cache`; a
/// matching `If-None-Match` is `304`.
///
/// Get meet names as they are listed by copying exact case-sensitive names from BARS
///
/// A blank `meet` is `400` for a 6.2.0+ client and `200 []` for a legacy one.
///
/// `start_time` and `weigh_in_time` are free text copied from the meet's published schedule,
/// not a normalized clock: ingest has stored `"08:00:00"`, `"10:00"`, and `"10:00 AM"`.
/// The app parses `h:mm[:ss][ AM/PM]`.
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
    client: ClientVersion,
    headers: HeaderMap,
    Query(params): Query<MeetsParams>,
) -> Result<Response, AppError> {
    client.require_non_empty("meet", &params.meet)?;
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

    cacheable_json(&rows, &headers)
}
