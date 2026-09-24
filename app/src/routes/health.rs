use crate::AppState;
use axum::{Json, extract::State};
use serde_json::{Value, json};

/// /health endpoint
///
/// curl 'https://api.meetcal.app/health' | jq .
///
/// Liveness plus the Postgres pool gauges, so an operator (or Uptime Kuma) can
/// see saturation without shell access: `size` is open connections, `idle` the
/// ones not serving a request. It deliberately does not round-trip to Postgres,
/// so a health probe never competes with real traffic for a connection.
///
/// {
///   "status": "ok",
///   "db": { "size": 4, "idle": 3 }
/// }
pub async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "db": {
            "size": state.db.size(),
            "idle": state.db.num_idle(),
        }
    }))
}
