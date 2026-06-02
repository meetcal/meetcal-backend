use crate::{
    AppError, AppState, common::query_convex::get_convex_response, routes::meets::types::Meets,
};
use axum::{
    Json,
    extract::{Path, State},
};
use convex::Value;
use std::collections::BTreeMap;

/// /meets/{name} endpoint
///
/// curl 'localhost:3000/meets/2026%20Ohio%20WSO%20Championships' | jq .
///
/// This endpoint takes the name of the meet exactly as it shows in BARS and returns the details of
/// the meet
///
/// {
///   "endDate": "2026-08-16",
///   "name": "2026 Ohio WSO Championships",
///   "startDate": "2026-08-15",
///   "timeZone": "America/New_York",
///   "venueCity": "Columbus",
///   "venueName": "2026 Ohio WSO Championships",
///   "venueState": "OH",
///   "venueStreet": "400 North High Street",
///   "venueZip": "43215"
/// }
pub async fn get_meet_details(
    State(state): State<AppState>,
    Path(meet_name): Path<String>,
) -> Result<Json<Meets>, AppError> {
    let mut args = BTreeMap::new();
    args.insert("name".to_string(), Value::from(meet_name));

    let response: Meets = get_convex_response(&state.convex, "meets:getByName", args).await?;

    Ok(Json(Meets {
        name: response.name,
        start_date: response.start_date,
        end_date: response.end_date,
        time_zone: response.time_zone,
        venue_city: response.venue_city,
        venue_name: response.venue_name,
        venue_state: response.venue_state,
        venue_street: response.venue_street,
        venue_zip: response.venue_zip,
    }))
}
