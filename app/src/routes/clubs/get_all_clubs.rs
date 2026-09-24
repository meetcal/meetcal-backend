use crate::{AppError, AppState, common::http_cache::cacheable_json};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;

/// /meets endpoint
///
/// curl 'https://api.meetcal.app/clubs' | jq .
///
/// This endpoint takes no input and returns a list of clubs in the db. The body
/// carries a strong `ETag` and `Cache-Control: no-cache`; a matching
/// `If-None-Match` is `304`.
///
/// [
///    "12 Labours Barbell",
///    "1Kilo",
///    "206 Barbell",
///    "3 Kings Weightlifting Club",
///    "351 Barbell Club",
///    "3P Weightlifting",
///    "4 Star Strength",
///    "5150 weightlifting",
///    "ALLSOUTH Barbell",
///    "ALPHA BARBELL",
/// ]
pub async fn get_all_clubs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let names: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT club
        FROM athletes
        WHERE club IS NOT NULL AND club <> ''
        ORDER BY club
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    let clubs: Vec<String> = names.into_iter().map(|(club,)| club).collect();

    cacheable_json(&clubs, &headers)
}
