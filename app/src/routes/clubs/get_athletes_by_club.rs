use crate::{AppError, AppState};
use axum::extract::State;
use axum::{Json, extract::Query};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct ClubsAthletesParams {
    pub meet: String,
    pub club: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ClubsAthletes {
    pub name: String,
    pub meet: String,
    pub club: String,
    pub gender: String,
    pub weight_class: String,
    pub entry_total: String,
}

/// /clubs/athletes endpoint
///
/// curl 'https://api.meetcal.app/clubs/athletes?meet=2026%20Ohio%20WSO%20Championships&club=Columbus%Weightlifting' | jq .
///
/// This endpoint takes meet name and club name and returns athletes from club in the meet
///
/// {
///  "athletes": [
///    {
///      "name": "Jane Doe",
///      "meet": "2024 Nationals",
///      "club": "ABC Weightlifting",
///      "gender": "Women",
///      "weight_class": "71kg",
///      "entry_total": 180
///    }
///  ]
/// }
pub async fn get_all_clubs(
    State(state): State<AppState>,
    Query(params): Query<ClubsAthletesParams>,
) -> Result<Json<Vec<ClubsAthletes>>, AppError> {
    let names: Vec<ClubsAthletes> = sqlx::query_as(
        r#"
        SELECT name, meet, club, gender, weight_class, entry_total
        FROM athletes
        WHERE club = $1 AND meet = $2
        ORDER BY name desc
        "#,
    )
    .bind(params.club)
    .bind(params.meet)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(names))
}
