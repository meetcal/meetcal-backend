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

/// Ceiling on `limit_per_name`. A lifter with more than this many results is
/// a long career; the app pages the rest through `/lifting-results/recent`.
pub const MAX_LIMIT_PER_NAME: u32 = 200;

#[derive(Debug, Deserialize, Serialize)]
pub struct ResultsByNamesParams {
    #[serde(deserialize_with = "deserialize_csv_or_repeated")]
    pub names: Vec<String>,
    pub latest_only: Option<bool>,
    pub limit_per_name: Option<u32>,
}

/// Every result for the requested names, newest first: the historical,
/// unbounded shape.
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

/// The bounded shape. `$2` (`latest_only`) keeps only rows on each name's most
/// recent date, which is that athlete's latest meet; `$3` (`limit_per_name`,
/// `NULL` for no cap) keeps the newest N rows per name. Both are per normalized
/// name, so two spellings of one lifter share a bound.
const BOUNDED_RESULTS_BY_NAMES_SQL: &str = concat!(
    r#"
        SELECT
            "#,
    lifting_result_columns!(),
    r#"
        FROM (
            SELECT
                *,
                ROW_NUMBER() OVER (
                    PARTITION BY "#,
    normalized_name_sql!(),
    r#"
                    ORDER BY date DESC, id DESC
                ) AS row_in_name,
                DENSE_RANK() OVER (
                    PARTITION BY "#,
    normalized_name_sql!(),
    r#"
                    ORDER BY date DESC
                ) AS date_rank
            FROM lifting_results
            WHERE "#,
    normalized_name_sql!(),
    r#" = ANY($1::text[])
        ) ranked
        WHERE (NOT $2::boolean OR date_rank = 1)
            AND ($3::bigint IS NULL OR row_in_name <= $3::bigint)
        ORDER BY date DESC
        "#
);

/// /lifting-results/by-names endpoint
///
/// curl 'https://api.meetcal.app/lifting-results/by-names?names=Alexander%20Nordstrom' | jq .
///
/// This endpoint takes an array of athlete names and returns the athletes' lifting results,
/// newest first. Without options it returns every row (unchanged for existing clients). Two
/// optional bounds, on the query string or in the `POST` body:
///
/// - `latest_only=true`: only the rows from each athlete's most recent meet date.
/// - `limit_per_name=N` (1..=200): at most N rows per athlete. Out of range is `400`.
///
/// [
///   {
///     "id": 1,
///     "event_id": "event_2025",
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
    results_by_names(
        &state,
        params.names,
        params.latest_only,
        params.limit_per_name,
    )
    .await
}

/// `POST /lifting-results/by-names` with `{"names": [...], "latest_only"?, "limit_per_name"?}`.
///
/// Same response as the `GET` form. The JSON array carries names verbatim, so a
/// name containing a comma is one name rather than two.
pub async fn post_results_by_names(
    State(state): State<AppState>,
    Json(body): Json<NameListBody>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    results_by_names(
        &state,
        clean_name_list(body.names),
        body.latest_only,
        body.limit_per_name,
    )
    .await
}

async fn results_by_names(
    state: &AppState,
    names: Vec<String>,
    latest_only: Option<bool>,
    limit_per_name: Option<u32>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    crate::common::query::require_name_list(&names)?;
    if let Some(limit) = limit_per_name
        && !(1..=MAX_LIMIT_PER_NAME).contains(&limit)
    {
        return Err(AppError::Validation(format!(
            "limit_per_name must be between 1 and {MAX_LIMIT_PER_NAME}"
        )));
    }
    let normalized_names: Vec<String> = names.iter().map(|name| normalize_name(name)).collect();
    let latest_only = latest_only.unwrap_or(false);

    let rows = if !latest_only && limit_per_name.is_none() {
        sqlx::query_as::<_, LiftingResults>(RESULTS_BY_NAMES_SQL)
            .bind(&normalized_names)
            .fetch_all(&state.db)
            .await?
    } else {
        sqlx::query_as::<_, LiftingResults>(BOUNDED_RESULTS_BY_NAMES_SQL)
            .bind(&normalized_names)
            .bind(latest_only)
            .bind(limit_per_name.map(i64::from))
            .fetch_all(&state.db)
            .await?
    };

    Ok(Json(rows))
}
