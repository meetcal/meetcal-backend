use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Deserialize, Serialize)]
pub struct IntlRankingsParams {
    pub meet: String,
    pub gender: String,
    pub age_category: String,
}

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
/// curl 'http://localhost:3000/data/intl-rankings?meet=Worlds&gender=Women&age_category=Junior' | jq .
///
/// This endpoint takes meet, gender, and age category and returns international rankings
///
/// Meets: Pan Ams, Worlds
/// Genders: Men, Women
/// Age Categories: U15, U17, Youth, Junior, University, Senior
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
    Query(params): Query<IntlRankingsParams>,
) -> Result<Json<Vec<IntlRanking>>, AppError> {
    let rows = sqlx::query_as::<_, IntlRanking>(
        r#"
        SELECT meet, ranking, name, weight_class, total, percent_a, gender, age_category
        FROM intl_rankings
        WHERE meet = $1
            AND gender = $2
            AND age_category = $3
        ORDER BY ranking desc
        "#,
    )
    .bind(params.meet)
    .bind(params.gender)
    .bind(params.age_category)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows))
}
