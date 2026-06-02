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
pub struct StandardParams {
    pub age_category: String,
    pub gender: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Standard {
    pub age_category: String,
    pub gender: String,
    pub standard_a: f64,
    pub standard_b: f64,
    pub weight_class: String,
}

/// /standards endpoint
///
/// curl 'http://localhost:3000/standards?gender=men&ageCategory=senior' | jq .
///
/// This endpoint takes gender and age category and returns standards
///
/// [
///  {
///    "ageCategory": "senior",
///    "gender": "men",
///    "standardA": 281.0,
///    "standardB": 267.0,
///    "weightClass": "60kg"
///  },
/// ]
pub async fn get_standards(
    State(state): State<AppState>,
    Query(record): Query<StandardParams>,
) -> Result<Json<Vec<Standard>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert(
        "ageCategory".to_string(),
        Value::from(record.age_category.to_lowercase()),
    );
    args.insert(
        "gender".to_string(),
        Value::from(record.gender.to_lowercase()),
    );

    let response: Vec<Standard> =
        get_convex_response(&state.convex, "standards:getFiltered", args).await?;

    let sorted = sort_by_class(response, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}
