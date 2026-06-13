use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Deserialize, Serialize)]
pub struct ResultsCurrentYearParams {
    pub name: String,
    pub cutoff_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct YearBests {
    pub best_snatch: f64,
    pub best_cj: f64,
    pub best_total: f64,
}

/// /lifting-results/year endpoint
///
/// curl 'https://api.meetcal.app/lifting-results/year?name=Adaptive%20Test%20Athlete&cutoff_date=2025-06-13' | jq .
///
/// This endpoint takes a name and optional cutoff_date and returns best lifts since that date. If
/// cutoff_date is omitted it defaults to the past year.
///
/// {
///   "best_snatch": 40.0,
///   "best_cj": 50.0,
///   "best_total": 90.0
/// }
///
pub async fn get_results_current_year(
    State(state): State<AppState>,
    Query(params): Query<ResultsCurrentYearParams>,
) -> Result<Json<YearBests>, AppError> {
    let rows = if let Some(cutoff_date) = params.cutoff_date {
        sqlx::query_as::<_, YearBests>(
            r#"
            SELECT
                COALESCE(MAX(GREATEST(
                    COALESCE(snatch_best, 0),
                    COALESCE(snatch1, 0),
                    COALESCE(snatch2, 0),
                    COALESCE(snatch3, 0)
                )), 0) AS best_snatch,
                COALESCE(MAX(GREATEST(
                    COALESCE(cj_best, 0),
                    COALESCE(cj1, 0),
                    COALESCE(cj2, 0),
                    COALESCE(cj3, 0)
                )), 0) AS best_cj,
                COALESCE(MAX(COALESCE(total, 0)), 0) AS best_total
            FROM lifting_results
            WHERE name = $1
                AND date >= $2
            "#,
        )
        .bind(params.name)
        .bind(cutoff_date)
        .fetch_one(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, YearBests>(
            r#"
            SELECT
                COALESCE(MAX(GREATEST(
                    COALESCE(snatch_best, 0),
                    COALESCE(snatch1, 0),
                    COALESCE(snatch2, 0),
                    COALESCE(snatch3, 0)
                )), 0) AS best_snatch,
                COALESCE(MAX(GREATEST(
                    COALESCE(cj_best, 0),
                    COALESCE(cj1, 0),
                    COALESCE(cj2, 0),
                    COALESCE(cj3, 0)
                )), 0) AS best_cj,
                COALESCE(MAX(COALESCE(total, 0)), 0) AS best_total
            FROM lifting_results
            WHERE name = $1
                AND date >= (CURRENT_DATE - INTERVAL '1 year')::date::text
            "#,
        )
        .bind(params.name)
        .fetch_one(&state.db)
        .await?
    };

    Ok(Json(rows))
}
