//! POST /users/me/rewards/{id}/claim — claim a reward month.
//!
//! iOS: returns an Apple promotional-offer signature bundle (applied at next
//! renewal). Android: defers the Google Play subscription renewal by 30 days.
//!
//! Both paths are idempotent: only a reward in `earned` status can be claimed,
//! and the status transition is guarded (`WHERE ... status = 'earned' RETURNING`)
//! so a concurrent or retried claim gets 409 rather than a second delivery. If
//! the relevant platform credentials are unconfigured, the endpoint returns 503
//! and leaves the reward untouched — it never fakes success.

use crate::{
    AppState,
    routes::referrals::play::{DeferRequest, PlayBilling},
    routes::users::auth::{set_backend_context, set_request_user},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
pub struct ClaimRequest {
    pub platform: String,
}

pub async fn claim_reward(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(reward_id): Path<i64>,
    Json(body): Json<ClaimRequest>,
) -> Response {
    let platform = body.platform.trim().to_ascii_lowercase();
    if platform != "ios" && platform != "android" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "platform must be 'ios' or 'android'" })),
        )
            .into_response();
    }

    let caller = match state.auth.user_id(&headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let result = if platform == "ios" {
        claim_ios(&state, &caller, reward_id).await
    } else {
        claim_android(&state, &caller, reward_id).await
    };

    match result {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}

/// Fetch the caller's reward under user context (RLS enforces ownership) and
/// verify it is claimable. Returns the tx with both user + backend context set
/// (backend needed to read subscription_events for the product lookup).
async fn open_claimable<'a>(
    state: &'a AppState,
    caller: &str,
    reward_id: i64,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, Response> {
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return Err(crate::AppError::from(e).into_response()),
    };
    if let Err(e) = set_request_user(&mut tx, caller).await {
        return Err(e.into_response());
    }

    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM reward_ledger WHERE id = $1 AND user_id = $2",
    )
    .bind(reward_id)
    .bind(caller)
    .fetch_optional(&mut *tx)
    .await;

    match status {
        Ok(Some(status)) if status == "earned" => {}
        Ok(Some(_)) => {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({ "error": "reward is not claimable" })),
            )
                .into_response());
        }
        Ok(None) => {
            return Err(
                (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
            );
        }
        Err(e) => return Err(crate::AppError::from(e).into_response()),
    }

    // Elevate to read the backend-only subscription_events for product lookup.
    if let Err(e) = set_backend_context(&mut tx).await {
        return Err(e.into_response());
    }

    Ok(tx)
}

/// Latest non-web product the caller is subscribed to (from the event audit log).
async fn latest_product(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    caller: &str,
) -> Result<Option<(String, Value)>, crate::AppError> {
    let row = sqlx::query_as::<_, (String, Value)>(
        r#"
        SELECT product_id, event FROM subscription_events
        WHERE app_user_id = $1
          AND product_id IS NOT NULL
          AND lower(coalesce(store, '')) NOT IN ('web', 'rc_billing')
        ORDER BY received_at DESC
        LIMIT 1
        "#,
    )
    .bind(caller)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row)
}

async fn claim_ios(
    state: &AppState,
    caller: &str,
    reward_id: i64,
) -> Result<Response, crate::AppError> {
    let Some(apple) = state.referrals.apple.clone() else {
        return Ok(not_configured(
            "apple in-app purchase signing is not configured",
        ));
    };

    let mut tx = match open_claimable(state, caller, reward_id).await {
        Ok(tx) => tx,
        Err(resp) => return Ok(resp),
    };

    let Some((product_id, _event)) = latest_product(&mut tx, caller).await? else {
        // No known store product to attach the offer to.
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "no_active_subscription" })),
        )
            .into_response());
    };

    let bundle = match apple.sign_offer(&product_id, caller) {
        Ok(bundle) => bundle,
        Err(crate::routes::referrals::apple::AppleError::NoOfferForProduct) => {
            return Ok(not_configured(
                "no promotional offer configured for product",
            ));
        }
        Err(crate::routes::referrals::apple::AppleError::KeyError) => {
            return Ok(not_configured("apple signing key is invalid"));
        }
    };

    // Guarded transition: only an `earned` reward becomes `claimed`.
    let updated = sqlx::query_scalar::<_, i64>(
        r#"
        UPDATE reward_ledger
        SET status = 'claimed', platform = 'ios', claimed_at = now(), updated_at = now()
        WHERE id = $1 AND user_id = $2 AND status = 'earned'
        RETURNING id
        "#,
    )
    .bind(reward_id)
    .bind(caller)
    .fetch_optional(&mut *tx)
    .await?;

    if updated.is_none() {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({ "error": "reward is not claimable" })),
        )
            .into_response());
    }

    tx.commit().await?;

    Ok((
        StatusCode::OK,
        Json(json!({
            "id": reward_id,
            "status": "claimed",
            "platform": "ios",
            "offer": bundle,
        })),
    )
        .into_response())
}

async fn claim_android(
    state: &AppState,
    caller: &str,
    reward_id: i64,
) -> Result<Response, crate::AppError> {
    let Some(play_cfg) = state.referrals.play.clone() else {
        return Ok(not_configured("google play delivery is not configured"));
    };

    let mut tx = match open_claimable(state, caller, reward_id).await {
        Ok(tx) => tx,
        Err(resp) => return Ok(resp),
    };

    let Some((product_id, event)) = latest_product(&mut tx, caller).await? else {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "no_active_subscription" })),
        )
            .into_response());
    };

    // Best-effort extraction of the Play purchase token + current expiry from the
    // RevenueCat event. Phase 2 MUST verify the authoritative source of the Play
    // purchase token (likely the RevenueCat REST subscriber API), not the webhook
    // payload, before this ships.
    let purchase_token = event
        .get("store_transaction_id")
        .or_else(|| event.get("transaction_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let current_expiry_ms = event
        .get("expiration_at_ms")
        .and_then(json_as_i64)
        .unwrap_or(0);

    if purchase_token.is_empty() || current_expiry_ms == 0 {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "cannot_resolve_play_subscription" })),
        )
            .into_response());
    }

    // Guarded transition earned -> delivering (claims the row; a concurrent claim
    // now sees a non-earned status and 409s).
    let updated = sqlx::query_scalar::<_, i64>(
        r#"
        UPDATE reward_ledger
        SET status = 'delivering', platform = 'android', claimed_at = now(), updated_at = now()
        WHERE id = $1 AND user_id = $2 AND status = 'earned'
        RETURNING id
        "#,
    )
    .bind(reward_id)
    .bind(caller)
    .fetch_optional(&mut *tx)
    .await?;

    if updated.is_none() {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({ "error": "reward is not claimable" })),
        )
            .into_response());
    }

    let billing = match play_cfg.billing() {
        Ok(b) => b,
        Err(_) => {
            // Bad creds: roll back so the reward stays claimable.
            return Ok(not_configured("google play credentials are invalid"));
        }
    };

    let req = DeferRequest {
        package_name: play_cfg.package_name.clone(),
        subscription_id: product_id,
        purchase_token,
        current_expiry_ms,
    };

    match billing.defer(&req).await {
        Ok(outcome) => {
            sqlx::query(
                r#"
                UPDATE reward_ledger
                SET status = 'delivered',
                    store_transaction_id = $2,
                    delivered_at = now(),
                    updated_at = now()
                WHERE id = $1
                "#,
            )
            .bind(reward_id)
            .bind(outcome.new_expiry_ms.to_string())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok((
                StatusCode::OK,
                Json(json!({
                    "id": reward_id,
                    "status": "delivered",
                    "platform": "android",
                    "store_transaction_id": outcome.new_expiry_ms.to_string(),
                })),
            )
                .into_response())
        }
        Err(e) => {
            // Roll the whole transaction back so the reward returns to `earned`
            // and can be retried. Never leave it stuck mid-delivery.
            drop(tx);
            Ok((
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "delivery_failed", "detail": e.to_string() })),
            )
                .into_response())
        }
    }
}

fn not_configured(message: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "not_configured", "detail": message })),
    )
        .into_response()
}

fn json_as_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}
