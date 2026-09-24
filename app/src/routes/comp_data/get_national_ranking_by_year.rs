use crate::{
    AppError, AppState,
    common::{client::ClientVersion, http_cache::cacheable_json},
};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Deserialize, Serialize)]
pub struct NatRankingsParamsYear {
    pub age_category: String,
    pub federation: String,
    pub year: String,
}

#[derive(Debug, Deserialize, Serialize, FromRow)]
pub struct NatRankingsYear {
    pub name: String,
    pub date: String,
    pub total: f64,
}

/// /data/nat-rankings-year endpoint
///
/// curl 'https://api.meetcal.app/data/nat-rankings-year?age_category=Open%20Men%27s%2060kg&federation=USAW&year=2026' | jq .
///
/// Age Categories:
///  "Men's 11 Under Age Group 32kg",
///  "Women's 11 Under Age Group 30kg",
///  "Men's 13 Under Age Group 32kg",
///  "Women's 13 Under Age Group 30kg",
///  "Men's 14-15 Age Group 48kg",
///  "Women's 14-15 Age Group 40kg",
///  "Men's 16-17 Age Group 56kg",
///  "Junior Men's 110+kg",
///  "Junior Women's 48kg",
///  "Open Men's 110+kg",
///  "Open Women's 48kg",
///  "Men's Masters (35-39) 110+kg",
///  "Women's Masters (35-39) 53kg",
///  "Men's Masters (40-44) 110+kg",
///  "Men's Masters (45-49) 110+kg",
///  "Men's Masters (50-54) 110+kg",
///  ...
///  "Men's Masters (80-84) 94kg",
/// Federations: USAW, USAMW
///
/// This endpoint takes federation, year, and age category and returns national rankings for a weight_class in a given year
///
/// `year` must be four digits for a 6.2.0+ client (`400` otherwise); a legacy client's malformed
/// year compares as text and returns `[]`, as it always has.
///
/// The body carries a strong `ETag` and `Cache-Control: no-cache`; a matching
/// `If-None-Match` is `304`.
///
/// [
///  {
///    "name": "gabe chhum",
///    "total": 281.0
///  },
///  {
///    "name": "Kaiden Mima",
///    "total": 247.0
///  },
/// ]
pub async fn get_national_rankings_by_year(
    State(state): State<AppState>,
    client: ClientVersion,
    headers: HeaderMap,
    Query(params): Query<NatRankingsParamsYear>,
) -> Result<Response, AppError> {
    client.require_year("year", &params.year)?;
    let year_start = format!("{}-01-01", params.year);
    let year_end = format!("{}-12-31", params.year);
    let rows = sqlx::query_as::<_, NatRankingsYear>(
        r#"
        SELECT name, COALESCE(total, 0) AS total, date
        FROM lifting_results
        WHERE federation = $1
            AND age = $2
            AND name IS NOT NULL
            AND total IS NOT NULL
            AND total <> 0
            AND date >= $3
            AND date <= $4
        "#,
    )
    .bind(&params.federation)
    .bind(&params.age_category)
    .bind(&year_start)
    .bind(&year_end)
    .fetch_all(&state.db)
    .await?;

    let rankings = super::best_total_per_athlete(rows, |row| row.name.as_str(), |row| row.total);

    cacheable_json(&rankings, &headers)
}
