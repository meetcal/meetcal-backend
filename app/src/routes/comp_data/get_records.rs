use crate::common::query_convex::get_convex_response;
use crate::common::sort::sort_by_class;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use convex::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordParams {
    pub age_category: String,
    pub gender: String,
    pub record_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
/// curl 'http://localhost:3000/records?recordType=USAW&gender=men&ageCategory=senior' | jq .
///
/// This endpoint takes federation, gender, and age category and returns records
///
/// TODO:
/// Federations: USAW, USAMW, IWF, UMWF
/// Genders: men, women
/// Age Categories:
///
/// [
///  {
///    "ageCategory": "senior",
///    "cjRecord": 185.0,
///    "gender": "men",
///    "recordType": "USAW",
///    "snatchRecord": 147.0,
///    "totalRecord": 339.0,
///    "weightClass": "71kg"
///  },
/// ]
pub async fn get_records(
    State(state): State<AppState>,
    Query(record): Query<RecordParams>,
) -> Result<Json<Vec<Record>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert(
        "ageCategory".to_string(),
        Value::from(record.age_category.to_lowercase()),
    );
    args.insert(
        "gender".to_string(),
        Value::from(record.gender.to_lowercase()),
    );
    args.insert(
        "recordType".to_string(),
        Value::from(record.record_type.to_uppercase()),
    );

    let response: Vec<Record> =
        get_convex_response(&state.convex, "records:getByFederation", args).await?;

    let sorted = sort_by_class(response, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}
