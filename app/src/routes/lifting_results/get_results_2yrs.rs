use crate::{
    AppError, AppState,
    common::{names::normalize_name, query::deserialize_csv_or_repeated},
    routes::results::types::LiftingResults,
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
    let normalized_names: Vec<String> = params.names.iter().map(|name| normalize_name(name)).collect();

    let rows = if let Some(cutoff_date) = params.cutoff_date {
        sqlx::query_as::<_, LiftingResults>(
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
                AND date >= $2
            ORDER BY date DESC
            "#,
        )
        .bind(&normalized_names)
        .bind(cutoff_date)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, LiftingResults>(
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
                AND date >= (CURRENT_DATE - INTERVAL '2 years')::date::text
            ORDER BY date DESC
            "#,
        )
        .bind(&normalized_names)
        .fetch_all(&state.db)
        .await?
    };

    Ok(Json(rows))
}
