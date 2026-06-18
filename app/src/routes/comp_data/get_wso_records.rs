use crate::common::sort::sort_by_class;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct WsoRecordParams {
    pub wso: String,
    pub age_category: Option<String>,
    pub gender: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WsoAgeGroupsParams {
    pub wso: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct WsoRecord {
    pub age_category: String,
    pub cj_record: f64,
    pub gender: String,
    pub snatch_record: f64,
    pub total_record: f64,
    pub weight_class: String,
    pub wso: String,
}

/// /data/wso/records endpoint
///
/// curl 'https://api.meetcal.app/data/wso/records?wso=Carolina&gender=Men&age_category=Senior' | jq .
///
/// This endpoint takes wso plus optional gender and age category filters and returns wso records
///
/// WSOs:
///    "California North",
///    "Carolina",
///    "DMV",
///    "Florida",
///    "Georgia",
///    "Illinois",
///    "Michigan",
///    "Minnesota-Dakotas",
///    "Mountain South",
///    "New England",
///    "New Jersey",
///    "New York",
///    "Ohio",
///    "Pacific Northwest",
///    "Pennsylvania-West Virginia",
///    "Tennessee-Kentucky",
///    "Texas-Oklahoma",
///    "Wisconsin"
/// Age Categories: U11, U13, U15, U17, Youth, Junior, Senior, Masters 35, Masters 40, ..., Masters 90
/// Gender: Men, Women
///
/// [
///  {
///    "age_category": "Senior",
///    "cj_record": 124.0,
///    "gender": "Men",
///    "snatch_record": 101.0,
///    "total_record": 225.0,
///    "weight_class": "60",
///    "wso": "Florida"
///  },
/// ]
pub async fn get_wso_records(
    State(state): State<AppState>,
    Query(params): Query<WsoRecordParams>,
) -> Result<Json<Vec<WsoRecord>>, AppError> {
    let rows = sqlx::query_as::<_, WsoRecord>(
        r#"
        SELECT age_category, cj_record, snatch_record, total_record, weight_class, gender, wso
        FROM wso_records
        WHERE wso = $1
            AND ($2::text IS NULL OR age_category = $2)
            AND ($3::text IS NULL OR gender = $3)
        "#,
    )
    .bind(params.wso)
    .bind(params.age_category)
    .bind(params.gender)
    .fetch_all(&state.db)
    .await?;

    let sorted = sort_by_class(rows, |r| r.weight_class.as_str());

    Ok(Json(sorted))
}

/// /data/wso/age-groups endpoint
///
/// curl 'https://api.meetcal.app/data/wso/age-groups?wso=Carolina' | jq .
///
/// This endpoint returns the age categories that have records for one WSO.
///
/// [
///   "U11",
///   "U13",
///   "Senior"
/// ]
pub async fn get_wso_age_groups(
    State(state): State<AppState>,
    Query(params): Query<WsoAgeGroupsParams>,
) -> Result<Json<Vec<String>>, AppError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT age_category
        FROM wso_records
        WHERE wso = $1
        ORDER BY age_category
        "#,
    )
    .bind(params.wso)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter().map(|(age_group,)| age_group).collect(),
    ))
}
