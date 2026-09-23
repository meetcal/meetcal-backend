use crate::common::sort::sort_by_class;
use crate::{AppError, AppState, common::http_cache::cacheable_json};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Record {
    pub age_category: String,
    pub gender: String,
    pub weight_class: String,
    pub record_type: String,
    pub snatch_record: f64,
    pub cj_record: f64,
    pub total_record: f64,
}

/// /data/records endpoint
///
/// curl 'https://api.meetcal.app/data/records' | jq .
///
/// The body carries a strong `ETag` and `Cache-Control: public, max-age=300`; a matching
/// `If-None-Match` is `304`.
///
/// This endpoint takes nothing and returns records
///
/// [
///  {
///    "age_category": "Senior",
///    "cj_record": 185.0,
///    "gender": "Men",
///    "record_type": "USAW",
///    "snatch_record": 147.0,
///    "total_record": 339.0,
///    "weight_class": "71kg"
///  },
/// ]
pub async fn get_records(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let rows = sqlx::query_as::<_, Record>(
        r#"
        SELECT age_category, cj_record, snatch_record, total_record, weight_class, gender, record_type
        FROM records
        WHERE snatch_record IS NOT NULL
            AND cj_record IS NOT NULL
            AND total_record IS NOT NULL
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    let sorted = sort_by_class(rows, |r| r.weight_class.as_str());

    cacheable_json(&sorted, &headers)
}
