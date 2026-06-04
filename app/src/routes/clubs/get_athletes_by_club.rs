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
    pub entry_total: f64,
}

/// /clubs/athletes endpoint
///
/// curl 'https://api.meetcal.app/clubs/athletes?&club=POWER%20AND%20GRACE%20PERFORMANCE%2E' | jq .
///
/// This endpoint takes club name and returns athletes from club in the completed meets
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
pub async fn get_athletes_by_club(
    State(state): State<AppState>,
    Query(params): Query<ClubsAthletesParams>,
) -> Result<Json<Vec<ClubsAthletes>>, AppError> {
    let names: Vec<ClubsAthletes> = sqlx::query_as(
        r#"
        SELECT name, meet, club, gender, weight_class, entry_total
        FROM athletes
        WHERE club = $1
            AND meet IN (
                SELECT name FROM meets WHERE status = 'completed'
            )
        ORDER BY name desc
        "#,
    )
    .bind(params.club)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(names))
}
