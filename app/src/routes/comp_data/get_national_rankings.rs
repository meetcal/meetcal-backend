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

/// /nat-rankings endpoint
///
/// curl 'http://localhost:3000/nat-rankings?ageCategory=Open%20Men%27s%2089kg&federation=USAW | jq .
///
/// TODO:
/// Age Categories:
/// Federations: USAW, USAMW
///
/// This endpoint takes federation and age category and returns national rankings for a weight_class
///
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
