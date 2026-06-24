use crate::{AppError, AppState, common::names::normalize_name, routes::results::types::LiftingResults};
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchParams {
    pub query: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchResponse {
    pub matched_name: Option<String>,
    pub suggestions: Vec<String>,
    pub results: Vec<LiftingResults>,
}

/// /search endpoint
///
/// Exact wrapped search:
/// curl 'https://api.meetcal.app/search?query=Alexander%20Nordstrom&start_date=2025-01-01&end_date=2025-12-31' | jq .
///
/// Name suggestions:
/// curl 'https://api.meetcal.app/search?query=Alexan' | jq .
///
/// This endpoint takes an athlete search query and returns a completed search payload. With
/// start_date and end_date it returns exact name results for the range when available, otherwise
/// fallback rows and suggestions. Without dates it returns name suggestions only.
///
/// {
///   "matched_name": "Alexander Nordstrom",
///   "suggestions": [],
///   "results": [
///     {
///       "federation": "USAW",
///       "meet": "2025 Test Meet",
///       "date": "2025-06-01",
///       "name": "Alexander Nordstrom",
///       "age": "Open Men's 60kg",
///       "body_weight": 59.9,
///       "snatch1": 90.0,
///       "snatch2": 95.0,
///       "snatch3": 100.0,
///       "snatch_best": 100.0,
///       "cj1": 120.0,
///       "cj2": 125.0,
///       "cj3": 130.0,
///       "cj_best": 130.0,
///       "total": 230.0,
///       "adaptive": false
///     }
///   ]
/// }
///
pub async fn search_wrapped(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>, AppError> {
    let suggestions = search_suggestions(&state, &params.query).await?;

    let (Some(start_date), Some(end_date)) = (params.start_date.as_ref(), params.end_date.as_ref())
    else {
        return Ok(Json(SearchResponse {
            matched_name: None,
            suggestions,
            results: Vec::new(),
        }));
    };

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
        WHERE lower(btrim(regexp_replace(name, '\s+', ' ', 'g'))) = $1 AND date >= $2 AND date < $3
        ORDER BY date ASC
        LIMIT 600
        "#,
    )
    .bind(normalize_name(&params.query))
    .bind(start_date)
    .bind(end_date)
    .fetch_all(&state.db)
    .await?;

    if !exact.is_empty() {
        return Ok(Json(SearchResponse {
            matched_name: Some(params.query),
            suggestions: Vec::new(),
            results: exact,
        }));
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
        LIMIT 600
        "#,
    )
    .bind(&pattern)
    .bind(start_date)
    .bind(end_date)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(SearchResponse {
        matched_name: None,
        suggestions,
        results: fallback,
    }))
}

async fn search_suggestions(state: &AppState, query: &str) -> Result<Vec<String>, AppError> {
    let pattern = format!("%{}%", query);

    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT name
        FROM lifting_results
        WHERE name ILIKE $1
        ORDER BY name
        LIMIT 8
        "#,
    )
    .bind(pattern)
    .fetch_all(&state.db)
    .await?;

    Ok(rows.into_iter().map(|(name,)| name).collect())
}
