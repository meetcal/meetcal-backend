use crate::common::sort::sort_by_class;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Deserialize, Serialize)]
pub struct StandardParams {
    pub age_category: String,
    pub gender: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Standard {
    pub age_category: String,
    pub gender: String,
    pub standard_a: f64,
    pub standard_b: f64,
    pub weight_class: String,
}

/// /data/standards endpoint
///
/// curl 'https://api.meetcal.app/data/standards?gender=Men&age_category=Senior' | jq .
///
/// This endpoint takes gender and age category and returns standards
///
/// Genders: Men, Women
/// Age Categories: U15, U17, Youth, Junior, Senior
///
/// [
///  {
///    "age_category": "Senior",
///    "gender": "Men",
///    "standard_a": 281.0,
///    "standard_b": 267.0,
///    "weight_class": "60kg"
///  },
/// ]
pub async fn get_standards(
    State(state): State<AppState>,
    Query(params): Query<StandardParams>,
) -> Result<Json<Vec<Standard>>, AppError> {
    let rows = sqlx::query_as::<_, Standard>(
        r#"
        SELECT age_category, gender, standard_a, standard_b, weight_class
        FROM standards
        WHERE gender = $1 AND age_category = $2
        "#,
    )
    .bind(params.gender)
    .bind(params.age_category)
    .fetch_all(&state.db)
    .await?;

    let sorted = sort_by_class(rows, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}
