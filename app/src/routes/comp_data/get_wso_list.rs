use crate::common::query_convex::get_convex_response;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    let mut response: Vec<String> =
        get_convex_response(&state.convex, "wsoRecords:listWsos", BTreeMap::new()).await?;

    response.sort();

    Ok(Json(WsoNames { wsos: response }))
}
