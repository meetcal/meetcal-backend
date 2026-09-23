use crate::{
    AppError, AppState,
    common::names::{normalize_name, normalized_name_sql},
    common::query::{like_contains_pattern, require_non_empty},
    routes::results::types::{LiftingResults, lifting_result_columns},
};
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};

/// Ceiling on rows returned by one wrapped search. A name can appear in
/// hundreds of meets, and the mobile client renders a list, so the row budget
/// is declared here once and bound into both range queries rather than typed
/// into each `LIMIT`.
const MAX_SEARCH_RESULT_ROWS: i64 = 600;
/// Ceiling on name suggestions offered for a partial query.
const MAX_SEARCH_SUGGESTIONS: i64 = 8;

const EXACT_NAME_IN_RANGE_SQL: &str = concat!(
    r#"
        SELECT
            "#,
    lifting_result_columns!(),
    r#"
        FROM lifting_results
        WHERE "#,
    normalized_name_sql!(),
    r#" = $1 AND date >= $2 AND date < $3
        ORDER BY date ASC
        LIMIT $4
        "#
);

const NAME_LIKE_IN_RANGE_SQL: &str = concat!(
    r#"
        SELECT
            "#,
    lifting_result_columns!(),
    r#"
        FROM lifting_results
        WHERE name ILIKE $1 AND date >= $2 AND date < $3
        ORDER BY date ASC
        LIMIT $4
        "#
);

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
    require_non_empty("query", &params.query)?;
    let suggestions = search_suggestions(&state, &params.query).await?;

    let (Some(start_date), Some(end_date)) = (params.start_date.as_ref(), params.end_date.as_ref())
    else {
        return Ok(Json(SearchResponse {
            matched_name: None,
            suggestions,
            results: Vec::new(),
        }));
    };

    let exact = sqlx::query_as::<_, LiftingResults>(EXACT_NAME_IN_RANGE_SQL)
        .bind(normalize_name(&params.query))
        .bind(start_date)
        .bind(end_date)
        .bind(MAX_SEARCH_RESULT_ROWS)
        .fetch_all(&state.db)
        .await?;

    if !exact.is_empty() {
        return Ok(Json(SearchResponse {
            matched_name: Some(params.query),
            suggestions: Vec::new(),
            results: exact,
        }));
    }

    let pattern = like_contains_pattern(&params.query);

    let fallback = sqlx::query_as::<_, LiftingResults>(NAME_LIKE_IN_RANGE_SQL)
        .bind(&pattern)
        .bind(start_date)
        .bind(end_date)
        .bind(MAX_SEARCH_RESULT_ROWS)
        .fetch_all(&state.db)
        .await?;

    Ok(Json(SearchResponse {
        matched_name: None,
        suggestions,
        results: fallback,
    }))
}

async fn search_suggestions(state: &AppState, query: &str) -> Result<Vec<String>, AppError> {
    let pattern = like_contains_pattern(query);

    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT name
        FROM lifting_results
        WHERE name ILIKE $1
        ORDER BY name
        LIMIT $2
        "#,
    )
    .bind(pattern)
    .bind(MAX_SEARCH_SUGGESTIONS)
    .fetch_all(&state.db)
    .await?;

    Ok(rows.into_iter().map(|(name,)| name).collect())
}
