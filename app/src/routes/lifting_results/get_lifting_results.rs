use crate::{
    AppError, AppState,
    routes::meets::types::MeetsParams,
    routes::results::types::{LiftingResults, lifting_result_columns},
};
use axum::Json;
use axum::extract::{Query, State};

const RESULTS_BY_MEET_SQL: &str = concat!(
    r#"
        SELECT
            "#,
    lifting_result_columns!(),
    r#"
        FROM lifting_results
        WHERE meet = $1
        ORDER BY name
        "#
);

/// /lifting-results endpoint
///
/// curl 'https://api.meetcal.app/lifting-results?meet=2025%20Test%20Meet' | jq .
///
/// This endpoint takes meet and returns the results
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
///
pub async fn get_lifting_results(
    State(state): State<AppState>,
    Query(params): Query<MeetsParams>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    crate::common::query::require_non_empty("meet", &params.meet)?;
    let rows = sqlx::query_as::<_, LiftingResults>(RESULTS_BY_MEET_SQL)
        .bind(params.meet)
        .fetch_all(&state.db)
        .await?;

    Ok(Json(rows))
}
