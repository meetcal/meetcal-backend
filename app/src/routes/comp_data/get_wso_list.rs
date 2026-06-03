use crate::{AppError, AppState};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct WsoNames {
    pub wsos: Vec<String>,
}

/// /wso endpoint
///
/// curl 'http://localhost:3000/wso' | jq .
///
/// This endpoint takes nothing and returns a list of wsos
///
/// Below is a list of all wsos that are in the db
/// Unlisted wsos do not follow USAW guidelines of having public records hosted online
///
/// {
///  "wsos": [
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
///  ]
/// }
pub async fn get_wso_list(State(state): State<AppState>) -> Result<Json<WsoNames>, AppError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT wso
        FROM wso_records
        ORDER BY wso
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(WsoNames {
        wsos: rows.into_iter().map(|(wso,)| wso).collect(),
    }))
}
