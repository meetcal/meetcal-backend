use crate::common::query_convex::get_convex_response;
use crate::routes::meets::types::Meets;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::State;
use chrono::{Local, Months, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct MeetsList3Months {
    pub names: Vec<String>,
}

const MONTHS_RANGE: u32 = 3;

/// /meets endpoint
///
/// curl 'localhost:3000/meets' | jq .
///
/// This endpoint takes no input and returns a list of meets in the db in the next 3 months sorted
/// by earlist to latest
///
/// "names": [
///   "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
///   "2026 Mountain North WSO Championships",
///   "2026 Ohio WSO Championships"
/// ]
pub async fn list_meets_next_3months(
    State(state): State<AppState>,
) -> Result<Json<MeetsList3Months>, AppError> {
    let response: Vec<Meets> =
        get_convex_response(&state.convex, "meets:listActive", BTreeMap::new()).await?;
    let mut within_3months: Vec<Meets> = response
        .into_iter()
        .filter(|meet| is_within_next_3_months(meet.start_date.as_str()).unwrap_or(false))
        .collect();

    within_3months.sort_by_key(|n| n.start_date.clone());

    Ok(Json(MeetsList3Months {
        names: within_3months.into_iter().map(|n| n.name).collect(),
    }))
}

fn is_within_next_3_months(date_str: &str) -> Result<bool, chrono::ParseError> {
    let now = Local::now().date_naive();

    let max_future_date = now
        .checked_add_months(Months::new(MONTHS_RANGE))
        .expect("Date arithmetic overflowed");

    let target_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")?;

    Ok(target_date >= now && target_date <= max_future_date)
}
