use crate::{
    AppError, AppState,
    common::{names::normalize_name, query::deserialize_csv_or_repeated},
    routes::results::types::LiftingResults,
};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ResultsByNamesParams {
    #[serde(deserialize_with = "deserialize_csv_or_repeated")]
    pub names: Vec<String>,
}

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
    let normalized_names: Vec<String> = params
        .names
        .iter()
        .map(|name| normalize_name(name))
        .collect();

    let rows = sqlx::query_as::<_, LiftingResults>(
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
        WHERE lower(btrim(regexp_replace(name, '\s+', ' ', 'g'))) = ANY($1::text[])
        ORDER BY date DESC
        "#,
    )
    .bind(&normalized_names)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows))
}
