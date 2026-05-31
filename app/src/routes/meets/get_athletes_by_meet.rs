use crate::{AppError, AppState, query_convex::get_convex_response, routes::meets::types::Athlete};
use axum::{
    Json,
    extract::{Path, State},
};
use convex::Value;
use std::collections::BTreeMap;

/// /meets/athletes/{name} endpoint
///
/// curl 'localhost:3000/meets/athletes/2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness' | jq .
///
/// This endpoint takes the name of the meet exactly as it shows in BARS and returns the athletes in
/// the meet
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
///  {
///    "adaptive": false,
///    "age": 31.0,
///    "club": "Unaffiliated",
///    "entryTotal": 390.0,
///    "gender": "Male",
///    "meet": "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
///    "memberId": "1009887",
///    "name": "Keiser Witte",
///    "sessionNumber": 45.0,
///    "sessionPlatform": "Red",
///    "weightClass": "+110",
///    "wso": null
///  }
///]
pub async fn get_athletes_by_meet(
    State(state): State<AppState>,
    Path(meet_name): Path<String>,
) -> Result<Json<Vec<Athlete>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert("meet".to_string(), Value::from(meet_name));

    let mut response: Vec<Athlete> =
        get_convex_response(&state.convex, "athletes:getByMeet", args).await?;

    response.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(response))
}
