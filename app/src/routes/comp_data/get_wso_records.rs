use crate::query_convex::get_convex_response;
use crate::sort::sort_by_class;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use convex::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsoRecordParams {
    pub age_category: String,
    pub gender: String,
    pub wso: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsoRecord {
    pub age_category: String,
    pub cj_record: f64,
    pub gender: String,
    pub snatch_record: f64,
    pub total_record: f64,
    pub weight_class: String,
    pub wso: String,
}
/// /wso-records/{name} endpoint
///
/// curl 'http://localhost:3000/wso-records?wso=Carolina&gender=Men&ageCategory=Senior' | jq .
///
/// This endpoint takes gender, wso, and age category and returns wso records
///
/// [
///  {
///    "ageCategory": "Senior",
///    "cjRecord": 124.0,
///    "gender": "Men",
///    "snatchRecord": 101.0,
///    "totalRecord": 225.0,
///    "weightClass": "60",
///    "wso": "Florida"
///  },
///  {
///    "ageCategory": "Senior",
///    "cjRecord": 147.0,
///    "gender": "Men",
///    "snatchRecord": 121.0,
///    "totalRecord": 264.0,
///    "weightClass": "65",
///    "wso": "Florida"
///  },
/// ]
pub async fn get_wso_records(
    State(state): State<AppState>,
    Query(params): Query<WsoRecordParams>,
) -> Result<Json<Vec<WsoRecord>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert("wso".to_string(), Value::from(params.wso));
    args.insert("ageCategory".to_string(), Value::from(params.age_category));
    args.insert("gender".to_string(), Value::from(params.gender));

    let response: Vec<WsoRecord> =
        get_convex_response(&state.convex, "wsoRecords:getByWso", args).await?;

    let sorted = sort_by_class(response, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}
