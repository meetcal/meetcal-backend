use crate::common::sort::sort_by_class;
use crate::{AppError, AppState, common::http_cache::cacheable_json};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Standard {
    pub age_category: String,
    pub gender: String,
    pub standard_a: f64,
    pub standard_b: f64,
    pub weight_class: String,
}

/// /data/standards endpoint
///
/// curl 'https://api.meetcal.app/data/standards' | jq .
///
/// The body carries a strong `ETag` and `Cache-Control: public, max-age=300`; a matching
/// `If-None-Match` is `304`.
///
/// This endpoint takes nothing and returns standards
///
/// [
///  {
///    "age_category": "Senior",
///    "gender": "Men",
///    "standard_a": 281.0,
///    "standard_b": 267.0,
///    "weight_class": "60kg"
///  },
/// ]
pub async fn get_standards(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let rows = sqlx::query_as::<_, Standard>(
        r#"
        SELECT age_category, gender, standard_a, standard_b, weight_class
        FROM standards
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    let sorted = sort_by_class(rows, |r| r.weight_class.as_str());

    cacheable_json(&sorted, &headers)
}
