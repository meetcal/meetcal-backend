use crate::{AppError, AppState};
use axum::extract::State;
use axum::{Json, extract::Query};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct ClubsAthletesParams {
    pub club: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ClubsAthletes {
    pub name: String,
    pub meet: String,
    pub club: String,
    pub gender: String,
    pub weight_class: String,
    pub member_id: String,
    pub entry_total: f64,
}

/// /clubs/athletes endpoint
///
/// curl 'https://api.meetcal.app/clubs/athletes?&club=POWER%20AND%20GRACE%20PERFORMANCE%2E' | jq .
///
/// This endpoint takes a club name and returns every known meet registration for that club.
///
/// Registrations must not be restricted by the current meet status. Historical reporting
/// uses this endpoint to associate results with the club an athlete represented at each meet,
/// and imported result meets are not guaranteed to have a matching `meets` status row.
///
/// {
///  "athletes": [
///    {
///      "name": "Jane Doe",
///      "member_id": "45678frtghyuji"
///      "meet": "2024 Nationals",
///      "club": "ABC Weightlifting",
///      "gender": "Women",
///      "weight_class": "71kg",
///      "entry_total": 180
///    }
///  ]
/// }
pub async fn get_athletes_by_club(
    State(state): State<AppState>,
    Query(params): Query<ClubsAthletesParams>,
) -> Result<Json<Vec<ClubsAthletes>>, AppError> {
    let names: Vec<ClubsAthletes> = sqlx::query_as(
        r#"
        SELECT name, meet, club, gender, weight_class, entry_total, member_id
        FROM athletes
        WHERE club = $1
        ORDER BY name desc
        "#,
    )
    .bind(params.club)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(names))
}
