use crate::{AppError, AppState, common::client::ClientVersion};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct WsoAthletesParams {
    pub wso: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct WsoAthlete {
    pub name: String,
    pub meet: String,
    pub club: String,
    pub wso: String,
    pub gender: String,
    pub weight_class: String,
    pub member_id: String,
    pub entry_total: f64,
}

/// Returns every known meet registration for a WSO.
///
/// Historical reports use the registration's WSO rather than an athlete's latest WSO, because
/// affiliation can change between meets.
///
/// A blank `wso` is `400` for a 6.2.0+ client and `200 []` for a legacy one.
pub async fn get_athletes_by_wso(
    State(state): State<AppState>,
    client: ClientVersion,
    Query(params): Query<WsoAthletesParams>,
) -> Result<Json<Vec<WsoAthlete>>, AppError> {
    client.require_non_empty("wso", &params.wso)?;
    let athletes = sqlx::query_as(
        r#"
        SELECT name, meet, club, wso, gender, weight_class, entry_total, member_id
        FROM athletes
        WHERE wso = $1
        ORDER BY name DESC
        "#,
    )
    .bind(params.wso)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(athletes))
}
