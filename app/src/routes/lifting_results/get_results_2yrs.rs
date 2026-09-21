use crate::{
    AppError, AppState,
    common::{
        names::{normalize_name, normalized_name_sql},
        query::deserialize_csv_or_repeated,
    },
    routes::results::types::{LiftingResults, lifting_result_columns},
};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Results2YrsParams {
    #[serde(deserialize_with = "deserialize_csv_or_repeated")]
    pub names: Vec<String>,
    pub cutoff_date: Option<String>,
}

const RESULTS_SINCE_CUTOFF_SQL: &str = concat!(
    r#"
            SELECT
                "#,
    lifting_result_columns!(),
    r#"
            FROM lifting_results
            WHERE "#,
    normalized_name_sql!(),
    r#" = ANY($1::text[])
                AND date >= $2
            ORDER BY date DESC
            "#
);

const RESULTS_LAST_2YRS_SQL: &str = concat!(
    r#"
            SELECT
                "#,
    lifting_result_columns!(),
    r#"
            FROM lifting_results
            WHERE "#,
    normalized_name_sql!(),
    r#" = ANY($1::text[])
                AND date >= (CURRENT_DATE - INTERVAL '2 years')::date::text
            ORDER BY date DESC
            "#
);

/// /lifting-results/recent endpoint
///
/// curl 'https://api.meetcal.app/lifting-results/recent?names=Adaptive%20Test%20Athlete' | jq .
///
/// This endpoint takes an array of names and returns result history since cutoff_date. If no
/// cutoff_date is provided it defaults to the last 2 years.
///
/// [
///   {
///     "federation": "USAW",
///     "meet": "2026 Adaptive Men 85kg National Championship",
///     "date": "2026-02-01",
///     "name": "Adaptive Test Athlete",
///     "age": "Adaptive Men 85kg",
///     "body_weight": 84.5,
///     "snatch1": 35.0,
///     "snatch2": 40.0,
///     "snatch3": 0.0,
///     "snatch_best": 40.0,
///     "cj1": 45.0,
///     "cj2": 50.0,
///     "cj3": 0.0,
///     "cj_best": 50.0,
///     "total": 90.0,
///     "adaptive": true
///   }
/// ]
///
pub async fn get_results_2yrs(
    State(state): State<AppState>,
    Query(params): Query<Results2YrsParams>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    crate::common::query::require_name_list(&params.names)?;
    crate::common::query::require_iso_date("cutoff_date", params.cutoff_date.as_deref())?;
    let normalized_names: Vec<String> = params
        .names
        .iter()
        .map(|name| normalize_name(name))
        .collect();

    let rows = if let Some(cutoff_date) = params.cutoff_date {
        sqlx::query_as::<_, LiftingResults>(RESULTS_SINCE_CUTOFF_SQL)
            .bind(&normalized_names)
            .bind(cutoff_date)
            .fetch_all(&state.db)
            .await?
    } else {
        sqlx::query_as::<_, LiftingResults>(RESULTS_LAST_2YRS_SQL)
            .bind(&normalized_names)
            .fetch_all(&state.db)
            .await?
    };

    Ok(Json(rows))
}
