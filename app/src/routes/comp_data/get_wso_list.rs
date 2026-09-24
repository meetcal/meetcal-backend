use crate::{AppError, AppState, common::http_cache::cacheable_json};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;

/// /data/wso/ endpoint
///
/// curl 'https://api.meetcal.app/data/wso/' | jq .
///
/// The body carries a strong `ETag` and `Cache-Control: no-cache`; a matching
/// `If-None-Match` is `304`.
///
/// This endpoint takes nothing and returns a list of wsos
///
/// Below is a list of all wsos that are in the db
/// Unlisted wsos do not follow USAW guidelines of having public records hosted online
///
/// [
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
/// ]
pub async fn get_wso_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT wso
        FROM wso_records
        ORDER BY wso
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    let wsos: Vec<String> = rows.into_iter().map(|(wso,)| wso).collect();

    cacheable_json(&wsos, &headers)
}
