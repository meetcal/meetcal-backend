use crate::common::sort::sort_by_class;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Deserialize, Serialize)]
pub struct QualifyingTotalsParams {
    pub event_name: String,
    pub gender: String,
    pub age_category: String,
}

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
/// curl 'http://localhost:3000/data/qualifying-totals?event_name=Nationals&gender=Men&age_category=Senior' | jq .
///
/// This endpoint takes event name, gender, and age category and returns qualifying totals
///
/// Names: Virus Series, Virus Finals, Nationals, Master's Pan Ams, IMWA Worlds
/// Genders: Men, Women
/// Age Categories: U11, U13, U15, U17, Youth, Junior, University, U25, Senior, Masters 35, Masters 40, ..., Masters 90
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
    Query(params): Query<QualifyingTotalsParams>,
) -> Result<Json<Vec<QualifyingTotal>>, AppError> {
    let rows = sqlx::query_as::<_, QualifyingTotal>(
        r#"
        SELECT event_name, gender, age_category, weight_class, qualifying_total
        FROM qualifying_totals
        WHERE event_name = $1 AND gender = $2 AND age_category = $3
        "#,
    )
    .bind(params.event_name)
    .bind(params.gender)
    .bind(params.age_category)
    .fetch_all(&state.db)
    .await?;

    let sorted = sort_by_class(rows, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}
