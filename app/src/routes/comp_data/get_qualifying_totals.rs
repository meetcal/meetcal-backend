use crate::common::sort::sort_by_class;
use crate::{AppError, AppState, common::http_cache::cacheable_json};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct QualifyingTotal {
    pub event_name: String,
    pub gender: String,
    pub age_category: String,
    pub weight_class: String,
    pub qualifying_total: f64,
}

/// /data/qualifying-totals endpoint
///
/// curl 'https://api.meetcal.app/data/qualifying-totals' | jq .
///
/// The body carries a strong `ETag` and `Cache-Control: no-cache`; a matching
/// `If-None-Match` is `304`.
///
/// This endpoint takes nothing and returns qualifying totals
///
/// [
///   {
///     "event_name": "Virus Finals",
///     "gender": "Women",
///     "age_category": "U11",
///     "weight_class": "30kg",
///     "qualifying_total": 30.0
///   }
/// ]
pub async fn get_qualifying_totals(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let rows = sqlx::query_as::<_, QualifyingTotal>(
        r#"
        SELECT event_name, gender, age_category, weight_class, qualifying_total
        FROM qualifying_totals
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    let sorted = sort_by_class(rows, |r| r.weight_class.as_str());

    cacheable_json(&sorted, &headers)
}
