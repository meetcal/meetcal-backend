use crate::{
    AppError, AppState,
    common::{client::ClientVersion, http_cache::cacheable_json},
    routes::meets::{
        get_all_meets::meet_columns,
        types::{Meets, MeetsParams},
    },
};
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::Response,
};

const MEET_DETAILS_SQL: &str = concat!(
    "SELECT ",
    meet_columns!(),
    r#"
        FROM meets
        WHERE name = $1
        "#
);

/// /meets/details endpoint
///
/// curl 'https://api.meetcal.app/meets/details?meet=2026%20Ohio%20WSO%20Championships' | jq .
///
/// This endpoint takes the name of the meet exactly as it shows in BARS and returns the details of
/// the meet. The body carries a strong `ETag` and `Cache-Control: no-cache`; a
/// matching `If-None-Match` is `304`.
///
/// Get meet names as they are listed by copying exact case-sensitive names from BARS
///
/// A blank `meet` is `400` for a 6.2.0+ client and `404` (no such meet) for a legacy one.
///
/// {
///   "id": "meet_ohio_2026",
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
    client: ClientVersion,
    headers: HeaderMap,
    Query(params): Query<MeetsParams>,
) -> Result<Response, AppError> {
    client.require_non_empty("meet", &params.meet)?;
    let meet = sqlx::query_as::<_, Meets>(MEET_DETAILS_SQL)
        .bind(params.meet)
        .fetch_one(&state.db)
        .await?;

    cacheable_json(&meet, &headers)
}
