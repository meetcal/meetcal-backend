use crate::common::query_convex::get_convex_response;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use convex::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntlRankingsParams {
    pub meet: String,
    pub gender: String,
    pub age_category: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntlRanking {
    pub meet: String,
    pub ranking: f64,
    pub name: String,
    pub weight_class: String,
    pub total: f64,
    pub percent_a: f64,
    pub gender: String,
    pub age_category: String,
}

/// /intl-rankings endpoint
///
/// curl 'http://localhost:3000/intl-rankings?meet=Worlds&gender=Women&ageCategory=Junior' | jq .
///
/// This endpoint takes meet, gender, and age category and returns international rankings
///
/// Meets: Pan Ams, Worlds
/// Genders: Men, Women
/// Age Categories: U15, U17, Youth, Junior, University, Senior
///
/// {
///   "rankings": [
///     {
///       "meet": "Worlds",
///       "ranking": 1.0,
///       "name": "Ella Nicholson",
///       "weightClass": "77",
///       "total": 245.0,
///       "percentA": 114.49,
///       "gender": "Women",
///       "ageCategory": "Junior"
///     }
///   ]
/// }
pub async fn get_intl_rankings(
    State(state): State<AppState>,
    Query(params): Query<IntlRankingsParams>,
) -> Result<Json<Vec<IntlRanking>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert("meet".to_string(), Value::from(params.meet));
    args.insert("gender".to_string(), Value::from(params.gender));
    args.insert("ageCategory".to_string(), Value::from(params.age_category));

    let response: Vec<IntlRanking> =
        get_convex_response(&state.convex, "intlRankings:getFiltered", args).await?;

    Ok(Json(response))
}
