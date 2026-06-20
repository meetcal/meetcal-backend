use crate::{AppError, AppState, common::query::deserialize_csv_or_repeated};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Serialize)]
pub struct ResultsCurrentYearParams {
    pub name: String,
    pub cutoff_date: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BatchYearBestsParams {
    #[serde(deserialize_with = "deserialize_csv_or_repeated")]
    pub names: Vec<String>,
    pub cutoff_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct YearBests {
    pub best_snatch: f64,
    pub best_cj: f64,
    pub best_total: f64,
}

#[derive(Debug, FromRow)]
struct YearBestsByName {
    name: String,
    best_snatch: f64,
    best_cj: f64,
    best_total: f64,
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

/// /lifting-results/bests endpoint
///
/// curl 'https://api.meetcal.app/lifting-results/bests?names=Adaptive%20Test%20Athlete' | jq .
///
/// This endpoint takes an array of names and returns best lifts by name since cutoff_date. If
/// cutoff_date is omitted it defaults to the past year.
///
/// {
///   "Adaptive Test Athlete": {
///     "best_snatch": 40.0,
///     "best_cj": 50.0,
///     "best_total": 90.0
///   }
/// }
///
pub async fn get_results_bests(
    State(state): State<AppState>,
    Query(params): Query<BatchYearBestsParams>,
) -> Result<Json<BTreeMap<String, YearBests>>, AppError> {
    let mut by_name: BTreeMap<String, YearBests> = params
        .names
        .iter()
        .map(|name| {
            (
                name.clone(),
                YearBests {
                    best_snatch: 0.0,
                    best_cj: 0.0,
                    best_total: 0.0,
                },
            )
        })
        .collect();

    if params.names.is_empty() {
        return Ok(Json(by_name));
    }

    let rows = if let Some(cutoff_date) = params.cutoff_date {
        sqlx::query_as::<_, YearBestsByName>(
            r#"
            SELECT
                name,
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
            WHERE name = ANY($1::text[])
                AND date >= $2
            GROUP BY name
            "#,
        )
        .bind(&params.names)
        .bind(cutoff_date)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, YearBestsByName>(
            r#"
            SELECT
                name,
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
            WHERE name = ANY($1::text[])
                AND date >= (CURRENT_DATE - INTERVAL '1 year')::date::text
            GROUP BY name
            "#,
        )
        .bind(&params.names)
        .fetch_all(&state.db)
        .await?
    };

    for row in rows {
        by_name.insert(
            row.name,
            YearBests {
                best_snatch: row.best_snatch,
                best_cj: row.best_cj,
                best_total: row.best_total,
            },
        );
    }

    Ok(Json(by_name))
}
