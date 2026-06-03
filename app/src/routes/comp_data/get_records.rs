use crate::common::sort::sort_by_class;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Deserialize, Serialize)]
pub struct RecordParams {
    pub age_category: String,
    pub gender: String,
    pub record_type: String,
}

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

/// /records endpoint
///
/// curl 'http://localhost:3000/records?record_type=USAW&gender=Men&age_category=Senior' | jq .
///
/// This endpoint takes federation, gender, and age category and returns records
///
/// Federations: USAW, USAMW, IWF, UMWF
/// Genders: Men, Women
/// Age Categories: U13, U15, U17, Youth, Junior, University, Senior, Masters 35, Masters 40, ..., Masters 90
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
    Query(param): Query<RecordParams>,
) -> Result<Json<Vec<Record>>, AppError> {
    let rows = sqlx::query_as::<_, Record>(
        r#"
        SELECT age_category, cj_record, snatch_record, total_record, weight_class, gender, record_type
        FROM records
        WHERE record_type = $1
            AND gender = $2
            AND age_category = $3
        "#,
    )
    .bind(param.record_type)
    .bind(param.gender)
    .bind(param.age_category)
    .fetch_all(&state.db)
    .await?;

    let sorted = sort_by_class(rows, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}
