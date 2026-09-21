use crate::{
    AppError, AppState,
    common::{
        names::{normalize_name, normalized_name_sql},
        query::deserialize_csv_or_repeated,
    },
    routes::results::types::best_lifts_columns,
};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::{BTreeMap, HashMap};

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

const BESTS_SINCE_CUTOFF_SQL: &str = concat!(
    r#"
            SELECT
                "#,
    best_lifts_columns!(),
    r#"
            FROM lifting_results
            WHERE "#,
    normalized_name_sql!(),
    r#" = $1
                AND date >= $2
            "#
);

const BESTS_LAST_YEAR_SQL: &str = concat!(
    r#"
            SELECT
                "#,
    best_lifts_columns!(),
    r#"
            FROM lifting_results
            WHERE "#,
    normalized_name_sql!(),
    r#" = $1
                AND date >= (CURRENT_DATE - INTERVAL '1 year')::date::text
            "#
);

const BATCH_BESTS_SINCE_CUTOFF_SQL: &str = concat!(
    r#"
            SELECT
                "#,
    normalized_name_sql!(),
    r#" AS name,
                "#,
    best_lifts_columns!(),
    r#"
            FROM lifting_results
            WHERE "#,
    normalized_name_sql!(),
    r#" = ANY($1::text[])
                AND date >= $2
            GROUP BY "#,
    normalized_name_sql!(),
    r#"
            "#
);

const BATCH_BESTS_LAST_YEAR_SQL: &str = concat!(
    r#"
            SELECT
                "#,
    normalized_name_sql!(),
    r#" AS name,
                "#,
    best_lifts_columns!(),
    r#"
            FROM lifting_results
            WHERE "#,
    normalized_name_sql!(),
    r#" = ANY($1::text[])
                AND date >= (CURRENT_DATE - INTERVAL '1 year')::date::text
            GROUP BY "#,
    normalized_name_sql!(),
    r#"
            "#
);

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
    crate::common::query::require_non_empty("name", &params.name)?;
    crate::common::query::require_iso_date("cutoff_date", params.cutoff_date.as_deref())?;
    let rows = if let Some(cutoff_date) = params.cutoff_date {
        sqlx::query_as::<_, YearBests>(BESTS_SINCE_CUTOFF_SQL)
            .bind(normalize_name(&params.name))
            .bind(cutoff_date)
            .fetch_one(&state.db)
            .await?
    } else {
        sqlx::query_as::<_, YearBests>(BESTS_LAST_YEAR_SQL)
            .bind(normalize_name(&params.name))
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
    crate::common::query::require_name_list(&params.names)?;
    crate::common::query::require_iso_date("cutoff_date", params.cutoff_date.as_deref())?;
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

    // Match results to the originally requested names case- and whitespace-insensitively,
    // while keeping the response keyed by the requested names the caller looks up by.
    let normalized_names: Vec<String> = params
        .names
        .iter()
        .map(|name| normalize_name(name))
        .collect();
    let mut requested_by_normalized: HashMap<String, Vec<String>> = HashMap::new();
    for name in &params.names {
        requested_by_normalized
            .entry(normalize_name(name))
            .or_default()
            .push(name.clone());
    }

    let rows = if let Some(cutoff_date) = params.cutoff_date {
        sqlx::query_as::<_, YearBestsByName>(BATCH_BESTS_SINCE_CUTOFF_SQL)
            .bind(&normalized_names)
            .bind(cutoff_date)
            .fetch_all(&state.db)
            .await?
    } else {
        sqlx::query_as::<_, YearBestsByName>(BATCH_BESTS_LAST_YEAR_SQL)
            .bind(&normalized_names)
            .fetch_all(&state.db)
            .await?
    };

    // `row.name` is the normalized form; fan it back out to every requested name
    // that normalizes to it.
    for row in rows {
        if let Some(requested) = requested_by_normalized.get(&row.name) {
            for name in requested {
                by_name.insert(
                    name.clone(),
                    YearBests {
                        best_snatch: row.best_snatch,
                        best_cj: row.best_cj,
                        best_total: row.best_total,
                    },
                );
            }
        }
    }

    Ok(Json(by_name))
}
