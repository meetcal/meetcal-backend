use crate::{AppError, AppState};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct IntlRanking {
    pub meet: String,
    pub ranking: f64,
    pub name: String,
    pub weight_class: String,
    pub total: f64,
    pub percent_a: f64,
    pub gender: String,
    pub age_category: String,
}

/// /data/intl-rankings endpoint
///
/// curl 'https://api.meetcal.app/data/intl-rankings' | jq .
///
/// This endpoint takes nothing and returns international rankings
///
/// [
///   {
///     "meet": "Worlds",
///     "ranking": 1.0,
///     "name": "Ella Nicholson",
///     "weight_class": "77",
///     "total": 245.0,
///     "percent_a": 114.49,
///     "gender": "Women",
///     "age_category": "Junior"
///   }
/// ]
pub async fn get_intl_rankings(
    State(state): State<AppState>,
) -> Result<Json<Vec<IntlRanking>>, AppError> {
    let rows = sqlx::query_as::<_, IntlRanking>(
        r#"
        SELECT meet, ranking, name, weight_class, total, percent_a, gender, age_category
        FROM intl_rankings
        ORDER BY ranking desc
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows))
}
