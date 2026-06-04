use crate::{AppError, AppState, routes::results::types::LiftingResults};
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchParams {
    pub query: String,
    pub start_date: String,
    pub end_date: String,
}

/// /search endpoint
///
/// gives all results for fully matched name for date range
/// curl 'https://api.meetcal.app/search?query=Alexander%20Nordstrom&start_date=2025-01-01&end_date=2025-12-31' | jq .
///
/// gives results for all athletes in range whose name matches characters of query
/// curl 'https://api.meetcal.app/search?query=Alexan&start_date=2025-01-01&end_date=2025-12-31' | jq .
///
/// This endpoint takes the the athletes name, start and end dates and returns their results for the
/// time frame for their wrapped. If there is no exact match we fallback to matching characters in
/// name and send that to app to show user options
///
/// [
///  {
///    "federation": "USAW",
///    "meet": "2025 Virus Weightlifting Series 2, Powered by Rogue Fitness",
///    "date": "2025-08-31",
///    "name": "Alexander Nordstrom",
///    "age": "Open Men's 110kg",
///    "body_weight": 100.35,
///    "snatch1": 115.0,
///    "snatch2": 120.0,
///    "snatch3": 125.0,
///    "snatch_best": 125.0,
///    "cj1": 145.0,
///    "cj2": 150.0,
///    "cj3": 155.0,
///    "cj_best": 155.0,
///    "total": 280.0,
///    "adaptive": false
///  }
/// ]
///
/// TODO: trim query; word-filter fallback rows; sort by date asc; cap 600
/// TODO: optional `mode=exact|suggest` or separate route for name autocomplete (`athletes.searchByName`)
/// TODO: dedupe fallback to distinct names when returning options for client picker
/// TODO: remove redundant clones — build fallback_args only inside empty branch
pub async fn search_wrapped(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    let exact = sqlx::query_as::<_, LiftingResults>(
        r#"
        SELECT
            COALESCE(federation, '') AS federation,
            meet,
            date,
            name,
            COALESCE(age, '') AS age,
            COALESCE(body_weight, 0) AS body_weight,
            COALESCE(snatch1, 0) AS snatch1,
            COALESCE(snatch2, 0) AS snatch2,
            COALESCE(snatch3, 0) AS snatch3,
            COALESCE(snatch_best, 0) AS snatch_best,
            COALESCE(cj1, 0) AS cj1,
            COALESCE(cj2, 0) AS cj2,
            COALESCE(cj3, 0) AS cj3,
            COALESCE(cj_best, 0) AS cj_best,
            COALESCE(total, 0) AS total,
            adaptive
        FROM lifting_results
        WHERE name = $1 AND date >= $2 AND date < $3
        ORDER BY date DESC
        "#,
    )
    .bind(&params.query)
    .bind(&params.start_date)
    .bind(&params.end_date)
    .fetch_all(&state.db)
    .await?;

    if !exact.is_empty() {
        return Ok(Json(exact));
    }

    let pattern = format!("%{}%", params.query);

    let fallback = sqlx::query_as::<_, LiftingResults>(
        r#"
        SELECT
            COALESCE(federation, '') AS federation,
            meet,
            date,
            name,
            COALESCE(age, '') AS age,
            COALESCE(body_weight, 0) AS body_weight,
            COALESCE(snatch1, 0) AS snatch1,
            COALESCE(snatch2, 0) AS snatch2,
            COALESCE(snatch3, 0) AS snatch3,
            COALESCE(snatch_best, 0) AS snatch_best,
            COALESCE(cj1, 0) AS cj1,
            COALESCE(cj2, 0) AS cj2,
            COALESCE(cj3, 0) AS cj3,
            COALESCE(cj_best, 0) AS cj_best,
            COALESCE(total, 0) AS total,
            adaptive
        FROM lifting_results
        WHERE name ILIKE $1 AND date >= $2 AND date < $3
        ORDER BY date ASC
        LIMIT 256
        "#,
    )
    .bind(&pattern)
    .bind(&params.start_date)
    .bind(&params.end_date)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(fallback))
}
