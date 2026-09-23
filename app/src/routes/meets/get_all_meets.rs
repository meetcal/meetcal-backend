use crate::{AppError, AppState, common::http_cache::cacheable_json, routes::meets::types::Meets};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;

/// The `meets` projection every meet endpoint returns; `id` is the stable
/// `convex_id` the package endpoint also uses.
macro_rules! meet_columns {
    () => {
        "convex_id AS id, name, start_date::text AS start_date, end_date::text AS end_date,
        time_zone, venue_city, venue_state, venue_name, venue_street, venue_zip,
        federation, status, venue_map_pdf_url, venue_map_apple_url"
    };
}
pub(crate) use meet_columns;

/// Meets that have not been marked completed and start within three months of
/// today *in the meet's own time zone*. `meet_local_date` (migration
/// `20260923100003`) is `NOW() AT TIME ZONE time_zone` with a fallback to the
/// UTC date for a zone name Postgres rejects; the UTC date alone runs a day
/// ahead of every US venue each evening.
const UPCOMING_MEETS_SQL: &str = concat!(
    "SELECT ",
    meet_columns!(),
    r#"
        FROM meets
        WHERE status != 'completed'
            AND start_date <= meet_local_date(time_zone) + INTERVAL '3 months'
        ORDER BY start_date ASC
        "#
);

const COMPLETED_MEETS_SQL: &str = concat!(
    "SELECT ",
    meet_columns!(),
    r#"
        FROM meets
        WHERE status = 'completed'
        ORDER BY start_date DESC
        "#
);

/// /meets endpoint
///
/// curl 'https://api.meetcal.app/meets' | jq .
///
/// This endpoint takes no input and returns a list of meets in the db in the next 3 months sorted
/// by earlist to latest. The body carries a strong `ETag` and
/// `Cache-Control: public, max-age=300`; a matching `If-None-Match` is `304`.
///
/// [
///   {
///     "id": "meet_ohio_2026",
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
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let rows = sqlx::query_as::<_, Meets>(UPCOMING_MEETS_SQL)
        .fetch_all(&state.db)
        .await?;

    cacheable_json(&rows, &headers)
}

/// Returns completed meets, newest first, for result and team reporting views.
pub async fn list_completed_meets(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let rows = sqlx::query_as::<_, Meets>(COMPLETED_MEETS_SQL)
        .fetch_all(&state.db)
        .await?;

    cacheable_json(&rows, &headers)
}
