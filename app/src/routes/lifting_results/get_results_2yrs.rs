use crate::{
    AppError, AppState,
    common::{
        client::ClientVersion,
        names::{normalize_name, normalized_name_sql},
        query::{NameListBody, clean_name_list, deserialize_csv_or_repeated},
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

/// `$2` is the caller's cutoff; `NULL` falls back to the legacy two-year window
/// computed in Postgres so the default is unchanged for clients that omit it.
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
                AND date >= COALESCE($2::text, (CURRENT_DATE - INTERVAL '2 years')::date::text)
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
    client: ClientVersion,
    Query(params): Query<Results2YrsParams>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    results_since_cutoff(&state, client, params.names, params.cutoff_date).await
}

/// `POST /lifting-results/recent` with `{"names": [...], "cutoff_date": "YYYY-MM-DD"}`.
pub async fn post_results_2yrs(
    State(state): State<AppState>,
    client: ClientVersion,
    Json(body): Json<NameListBody>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    results_since_cutoff(
        &state,
        client,
        clean_name_list(body.names),
        body.cutoff_date,
    )
    .await
}

async fn results_since_cutoff(
    state: &AppState,
    client: ClientVersion,
    names: Vec<String>,
    cutoff_date: Option<String>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    crate::common::query::require_name_list(&names)?;
    client.require_present_iso_date("cutoff_date", cutoff_date.as_deref())?;
    let normalized_names: Vec<String> = names.iter().map(|name| normalize_name(name)).collect();

    let rows = sqlx::query_as::<_, LiftingResults>(RESULTS_SINCE_CUTOFF_SQL)
        .bind(&normalized_names)
        .bind(cutoff_date)
        .fetch_all(&state.db)
        .await?;

    Ok(Json(rows))
}
