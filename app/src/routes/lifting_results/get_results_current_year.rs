use crate::{
    AppError, AppState,
    common::{
        client::ClientVersion,
        names::{normalize_name, normalized_name_sql},
        query::{NameListBody, clean_name_list, deserialize_csv_or_repeated},
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

/// `$2` is the caller's cutoff; `NULL` falls back to the legacy one-year window
/// computed in Postgres so the default is unchanged for clients that omit it.
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
                AND date >= COALESCE($2::text, (CURRENT_DATE - INTERVAL '1 year')::date::text)
            "#
);

/// Same fallback as [`BESTS_SINCE_CUTOFF_SQL`], for a list of names.
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
                AND date >= COALESCE($2::text, (CURRENT_DATE - INTERVAL '1 year')::date::text)
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
    client: ClientVersion,
    Query(params): Query<ResultsCurrentYearParams>,
) -> Result<Json<YearBests>, AppError> {
    crate::common::query::require_non_empty("name", &params.name)?;
    client.require_present_iso_date("cutoff_date", params.cutoff_date.as_deref())?;
    let rows = sqlx::query_as::<_, YearBests>(BESTS_SINCE_CUTOFF_SQL)
        .bind(normalize_name(&params.name))
        .bind(params.cutoff_date)
        .fetch_one(&state.db)
        .await?;

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
    client: ClientVersion,
    Query(params): Query<BatchYearBestsParams>,
) -> Result<Json<BTreeMap<String, YearBests>>, AppError> {
    bests_by_name(&state, client, params.names, params.cutoff_date).await
}

/// `POST /lifting-results/bests` with `{"names": [...], "cutoff_date": "YYYY-MM-DD"}`.
///
/// Same response as the `GET` form, keyed by the requested names. The JSON
/// array carries names verbatim, so a name containing a comma is one key.
pub async fn post_results_bests(
    State(state): State<AppState>,
    client: ClientVersion,
    Json(body): Json<NameListBody>,
) -> Result<Json<BTreeMap<String, YearBests>>, AppError> {
    bests_by_name(
        &state,
        client,
        clean_name_list(body.names),
        body.cutoff_date,
    )
    .await
}

async fn bests_by_name(
    state: &AppState,
    client: ClientVersion,
    names: Vec<String>,
    cutoff_date: Option<String>,
) -> Result<Json<BTreeMap<String, YearBests>>, AppError> {
    crate::common::query::require_name_list(&names)?;
    client.require_present_iso_date("cutoff_date", cutoff_date.as_deref())?;
    let mut by_name: BTreeMap<String, YearBests> = names
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
    let normalized_names: Vec<String> = names.iter().map(|name| normalize_name(name)).collect();
    let mut requested_by_normalized: HashMap<String, Vec<String>> = HashMap::new();
    for name in &names {
        requested_by_normalized
            .entry(normalize_name(name))
            .or_default()
            .push(name.clone());
    }

    let rows = sqlx::query_as::<_, YearBestsByName>(BATCH_BESTS_SINCE_CUTOFF_SQL)
        .bind(&normalized_names)
        .bind(cutoff_date)
        .fetch_all(&state.db)
        .await?;

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
