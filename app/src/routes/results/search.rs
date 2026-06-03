use crate::{
    AppError, AppState, common::query_convex::get_convex_response,
    routes::comp_data::types::LiftingResults,
};
use axum::{
    Json,
    extract::{Query, State},
};
use convex::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    pub query: String,
    pub start_date: String,
    pub end_date: String,
}

/// /search endpoint
///
/// gives all results for fully matched name for date range
/// curl 'localhost:3000/search?query=Alexander%20Nordstrom&startDate=2025-01-01&endDate=2025-12-31' | jq .
///
/// gives results for all athletes in range whose name matches characters of query
/// curl 'localhost:3000/search?query=Alexan&startDate=2025-01-01&endDate=2025-12-31' | jq .
///
/// This endpoint takes the the athletes name, start and end dates and returns their results for the
/// time frame for their wrapped. If there is no exact match we fallback to matching characters in
/// name and send that to app to show user options
///
/// [
///  {
///    "federation": "USAW",
///    "meet": "2025 Virus Weightlifting Series 2, Powered by Rogue Fitness",
///    "date": "2025-08-31",
///    "name": "Alexander Nordstrom",
///    "age": "Open Men's 110kg",
///    "bodyWeight": 100.35,
///    "snatch1": 115.0,
///    "snatch2": 120.0,
///    "snatch3": 125.0,
///    "snatchBest": 125.0,
///    "cj1": 145.0,
///    "cj2": 150.0,
///    "cj3": 155.0,
///    "cjBest": 155.0,
///    "total": 280.0,
///    "adaptive": false
///  }
/// ]
///
/// TODO: trim query; word-filter fallback rows; sort by date asc; cap 600
/// TODO: optional `mode=exact|suggest` or separate route for name autocomplete (`athletes.searchByName`)
/// TODO: dedupe fallback to distinct names when returning options for client picker
/// TODO: remove redundant clones — build fallback_args only inside empty branch
pub async fn search_wrapped(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<LiftingResults>>, AppError> {
    let end_date = params.end_date.clone();
    let start_date = params.start_date.clone();

    let mut init_args = BTreeMap::new();
    init_args.insert(
        "names".to_string(),
        Value::from(vec![Value::from(params.query.clone())]),
    );

    let mut fallback_args = BTreeMap::new();
    fallback_args.insert("query".to_string(), Value::from(params.query.clone()));
    fallback_args.insert(
        "startDate".to_string(),
        Value::from(params.start_date.clone()),
    );
    fallback_args.insert("endDate".to_string(), Value::from(params.end_date.clone()));

    let response: Vec<LiftingResults> =
        get_convex_response(&state.convex, "liftingResults:getByNames", init_args).await?;

    let filtered = response
        .into_iter()
        .filter(|row| row.date < end_date && row.date >= start_date)
        .collect::<Vec<LiftingResults>>();

    if filtered.is_empty() {
        let response: Vec<LiftingResults> = get_convex_response(
            &state.convex,
            "liftingResults:searchByNameAndYear",
            fallback_args,
        )
        .await?;

        Ok(Json(response))
    } else {
        Ok(Json(filtered))
    }
}
