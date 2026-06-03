use crate::{
    AppError, AppState,
    common::query_convex::get_convex_response,
    routes::meets::types::{Athlete, MeetsParams},
};
use axum::{
    Json,
    extract::{Query, State},
};
use convex::Value;
use std::collections::BTreeMap;

/// /meets/athletes/{name} endpoint
///
/// curl 'localhost:3000/meets/athletes?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness' | jq .
///
/// This endpoint takes the name of the meet exactly as it shows in BARS and returns the athletes in
/// the meet
///
/// Get meet names as they are listed by copying exact case-sensitive names from BARS
///
/// [
///  {
///    "adaptive": false,
///    "age": 27.0,
///    "club": "Vardanian Weightlifting",
///    "entryTotal": 365.0,
///    "gender": "Male",
///    "meet": "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
///    "memberId": "1043909",
///    "name": "Kyle Schulman",
///    "sessionNumber": 45.0,
///    "sessionPlatform": "Red",
///    "weightClass": "+110",
///    "wso": null
///  },
///]
pub async fn get_athletes_by_meet(
    State(state): State<AppState>,
    Query(params): Query<MeetsParams>,
) -> Result<Json<Vec<Athlete>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert("meet".to_string(), Value::from(params.meet));

    let mut response: Vec<Athlete> =
        get_convex_response(&state.convex, "athletes:getByMeet", args).await?;

    response.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(response))
}
