use crate::common::sort::sort_by_class;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct WsoRecordParams {
    pub age_category: String,
    pub gender: String,
    pub wso: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct WsoRecord {
    pub age_category: String,
    pub cj_record: f64,
    pub gender: String,
    pub snatch_record: f64,
    pub total_record: f64,
    pub weight_class: String,
    pub wso: String,
}

/// /data/wso/records endpoint
///
/// curl 'http://localhost:3000/data/wso/records?wso=Carolina&gender=Men&age_category=Senior' | jq .
///
/// This endpoint takes gender, wso, and age category and returns wso records
///
/// WSOs:
///    "California North",
///    "Carolina",
///    "DMV",
///    "Florida",
///    "Georgia",
///    "Illinois",
///    "Michigan",
///    "Minnesota-Dakotas",
///    "Mountain South",
///    "New England",
///    "New Jersey",
///    "New York",
///    "Ohio",
///    "Pacific Northwest",
///    "Pennsylvania-West Virginia",
///    "Tennessee-Kentucky",
///    "Texas-Oklahoma",
///    "Wisconsin"
/// Age Categories: U11, U13, U15, U17, Youth, Junior, Senior, Masters 35, Masters 40, ..., Masters 90
/// Gender: Men, Women
///
/// [
///  {
///    "age_category": "Senior",
///    "cj_record": 124.0,
///    "gender": "Men",
///    "snatch_record": 101.0,
///    "total_record": 225.0,
///    "weight_class": "60",
///    "wso": "Florida"
///  },
/// ]
pub async fn get_wso_records(
    State(state): State<AppState>,
    Query(params): Query<WsoRecordParams>,
) -> Result<Json<Vec<WsoRecord>>, AppError> {
    let rows = sqlx::query_as::<_, WsoRecord>(
        r#"
        SELECT age_category, cj_record, snatch_record, total_record, weight_class, gender, wso
        FROM wso_records
        WHERE age_category = $1
            AND gender = $2
            AND wso = $3
        "#,
    )
    .bind(params.age_category)
    .bind(params.gender)
    .bind(params.wso)
    .fetch_all(&state.db)
    .await?;

    let sorted = sort_by_class(rows, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}
