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
pub struct QualifyingTotalsParams {
    pub event_name: String,
    pub gender: String,
    pub age_category: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualifyingTotal {
    pub event_name: String,
    pub gender: String,
    pub age_category: String,
    pub weight_class: String,
    pub qualifying_total: f64,
}

/// /qualifying-totals endpoint
///
/// curl 'http://localhost:3000/qualifying-totals?eventName=Nationals&gender=Men&ageCategory=Senior' | jq .
///
/// This endpoint takes event name, gender, and age category and returns qualifying totals
///
/// Names: Virus Series, Virus Finals, Nationals, Master's Pan Ams, IMWA Worlds
/// Genders: Men, Women
/// Age Categories: U11, U13, U15, U17, Youth, Junior, University, U25, Senior, Masters 35, Masters 40, ..., Masters 90
///
/// {
///   "rows": [
///     {
///       "eventName": "Virus Finals",
///       "gender": "Women",
///       "ageCategory": "U11",
///       "weightClass": "30kg",
///       "qualifyingTotal": 30.0
///     }
///   ]
/// }
pub async fn get_qualifying_totals(
    State(state): State<AppState>,
    Query(params): Query<QualifyingTotalsParams>,
) -> Result<Json<Vec<QualifyingTotal>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert("eventName".to_string(), Value::from(params.event_name));
    args.insert("gender".to_string(), Value::from(params.gender));
    args.insert("ageCategory".to_string(), Value::from(params.age_category));

    let response: Vec<QualifyingTotal> =
        get_convex_response(&state.convex, "qualifyingTotals:getFiltered", args).await?;

    let sorted = sort_by_class(response, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}
