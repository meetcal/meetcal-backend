use crate::common::sort::sort_by_class;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct QualifyingTotal {
    pub event_name: String,
    pub gender: String,
    pub age_category: String,
    pub weight_class: String,
    pub qualifying_total: f64,
}

/// /data/qualifying-totals endpoint
///
/// curl 'https://api.meetcal.app/data/qualifying-totals' | jq .
///
/// This endpoint takes nothing and returns qualifying totals
///
/// [
///   {
///     "event_name": "Virus Finals",
///     "gender": "Women",
///     "age_category": "U11",
///     "weight_class": "30kg",
///     "qualifying_total": 30.0
///   }
/// ]
pub async fn get_qualifying_totals(
    State(state): State<AppState>,
) -> Result<Json<Vec<QualifyingTotal>>, AppError> {
    let rows = sqlx::query_as::<_, QualifyingTotal>(
        r#"
        SELECT event_name, gender, age_category, weight_class, qualifying_total
        FROM qualifying_totals
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    let sorted = sort_by_class(rows, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}
