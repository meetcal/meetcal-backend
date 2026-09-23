use crate::{
    AppError, AppState,
    common::{
        names::{normalize_name, normalized_name_sql},
        query::{NameListBody, clean_name_list, deserialize_csv_or_repeated},
    },
    routes::results::types::{LiftingResults, lifting_result_columns},
};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ResultsByNamesParams {
    #[serde(deserialize_with = "deserialize_csv_or_repeated")]
    pub names: Vec<String>,
}

const RESULTS_BY_NAMES_SQL: &str = concat!(
    r#"
        SELECT
            "#,
    lifting_result_columns!(),
    r#"
        FROM lifting_results
        WHERE "#,
    normalized_name_sql!(),
    r#" = ANY($1::text[])
        ORDER BY date DESC
        "#
);

/// /lifting-results/by-names endpoint
///
/// curl 'https://api.meetcal.app/lifting-results/by-names?names=Alexander%20Nordstrom' | jq .
///
/// This endpoint takes an array of athlete names and returns the athletes' lifting results
///
/// [
///   {
///     "federation": "USAW",
///     "meet": "2025 Test Meet",
///     "date": "2025-06-01",
///     "name": "Alexander Nordstrom",
///     "age": "Open Men's 60kg",
///     "body_weight": 59.9,
///     "snatch1": 90.0,
///     "snatch2": 95.0,
///     "snatch3": 100.0,
///     "snatch_best": 100.0,
///     "cj1": 120.0,
///     "cj2": 125.0,
///     "cj3": 130.0,
///     "cj_best": 130.0,
///     "total": 230.0,
///     "adaptive": false
///   }
/// ]
pub async fn get_results_by_names(
    State(state): State<AppState>,
    Query(params): Query<ResultsByNamesParams>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    results_by_names(&state, params.names).await
}

/// `POST /lifting-results/by-names` with `{"names": [...]}`.
///
/// Same response as the `GET` form. The JSON array carries names verbatim, so a
/// name containing a comma is one name rather than two.
pub async fn post_results_by_names(
    State(state): State<AppState>,
    Json(body): Json<NameListBody>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    results_by_names(&state, clean_name_list(body.names)).await
}

async fn results_by_names(
    state: &AppState,
    names: Vec<String>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    crate::common::query::require_name_list(&names)?;
    let normalized_names: Vec<String> = names.iter().map(|name| normalize_name(name)).collect();

    let rows = sqlx::query_as::<_, LiftingResults>(RESULTS_BY_NAMES_SQL)
        .bind(&normalized_names)
        .fetch_all(&state.db)
        .await?;

    Ok(Json(rows))
}
