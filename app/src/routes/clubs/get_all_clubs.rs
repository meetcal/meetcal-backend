use crate::{AppError, AppState};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Clubs {
    pub names: Vec<String>,
}

/// /meets endpoint
///
/// curl 'localhost:3000/clubs' | jq .
///
/// This endpoint takes no input and returns a list of clubs in the db
///
/// "names": [
///    "12 Labours Barbell",
///    "1Kilo",
///    "206 Barbell",
///    "3 Kings Weightlifting Club",
///    "351 Barbell Club",
///    "3P Weightlifting",
///    "4 Star Strength",
///    "5150 weightlifting",
///    "ALLSOUTH Barbell",
///    "ALPHA BARBELL",
/// ]
pub async fn get_all_clubs(State(state): State<AppState>) -> Result<Json<Clubs>, AppError> {
    let names: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT club
        FROM athletes
        WHERE club IS NOT NULL AND club <> ''
        ORDER BY club
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(Clubs {
        names: names.into_iter().map(|(club,)| club).collect(),
    }))
}
