//! POST /users/me/rewards/{id}/claim — claim a reward month.
//!
//! iOS: returns an Apple promotional-offer signature bundle (applied at next
//! renewal) WITHOUT consuming the reward. The reward stays `earned` and only
//! transitions to `delivered` when the RevenueCat webhook observes the promo
//! renewal actually applied (see `webhook::is_promo_redemption`). This avoids
//! permanently burning a reward if the user cancels the Apple purchase sheet.
//!
//! Android: transitions `earned` -> `delivering` (guarded, so a concurrent claim
//! gets 409), defers the Google Play renewal by 30 days, then `delivered`. On any
//! delivery error the whole transaction rolls back to `earned` (retryable).
//!
//! If the relevant platform credentials are unconfigured, the endpoint returns
//! 503 and leaves the reward untouched — it never fakes success.

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

    let bundle = match apple.sign_offer(&product_id) {
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

    // IMPORTANT: do NOT consume the reward here. The client runs the Apple
    // purchase after this; if it fails or is cancelled, the reward must remain
    // claimable. We only stamp an audit timestamp. Delivery (earned ->
    // delivered) is confirmed later by the RevenueCat webhook.
    //
    // Re-issuance hold (M2): only stamp/return a fresh signature when the reward
    // is `earned` AND no signature was issued within the last
    // IOS_OFFER_REISSUE_HOLD_HOURS. This bounds unlimited re-issuance while still
    // letting the client retry after the hold lapses (covers a failed purchase).
    let hold = format!(
        "{} hours",
        crate::routes::referrals::IOS_OFFER_REISSUE_HOLD_HOURS
    );
    let updated = sqlx::query_scalar::<_, i64>(
        r#"
        UPDATE reward_ledger
        SET ios_offer_issued_at = now(), updated_at = now()
        WHERE id = $1 AND user_id = $2 AND status = 'earned'
          AND (ios_offer_issued_at IS NULL
               OR ios_offer_issued_at < now() - $3::interval)
        RETURNING id
        "#,
    )
    .bind(reward_id)
    .bind(caller)
    .bind(&hold)
    .fetch_optional(&mut *tx)
    .await?;

    if updated.is_none() {
        // open_claimable already confirmed the reward exists and is `earned`, so
        // a miss here means a signature was issued within the hold window.
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({ "error": "offer_recently_issued" })),
        )
            .into_response());
    }

    tx.commit().await?;

    // Flattened, top-level response (the app reads these directly, not under
    // `.offer`). Status stays `earned`; the app keys off HTTP 200.
    Ok((
        StatusCode::OK,
        Json(json!({
            "id": reward_id,
            "status": "earned",
            "platform": "ios",
            "product_id": bundle.product_id,
            "offer_id": bundle.offer_id,
            "key_id": bundle.key_id,
            "nonce": bundle.nonce,
            "timestamp": bundle.timestamp,
            "signature": bundle.signature,
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
        purchase_token: purchase_token.clone(),
        current_expiry_ms,
    };

    match billing.defer(&req).await {
        Ok(outcome) => {
            // store_transaction_id = the Play purchase token (the real store
            // transaction identifier); the new expiry gets its own column.
            sqlx::query(
                r#"
                UPDATE reward_ledger
                SET status = 'delivered',
                    store_transaction_id = $2,
                    delivered_expiry_ms = $3,
                    delivered_at = now(),
                    updated_at = now()
                WHERE id = $1
                "#,
            )
            .bind(reward_id)
            .bind(&purchase_token)
            .bind(outcome.new_expiry_ms)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok((
                StatusCode::OK,
                Json(json!({
                    "id": reward_id,
                    "status": "delivered",
                    "platform": "android",
                    "store_transaction_id": purchase_token,
                    "new_expiry_ms": outcome.new_expiry_ms,
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

/// POST /users/me/rewards/{id}/release-offer — release an outstanding iOS
/// promotional-offer reservation.
///
/// The iOS claim stamps `ios_offer_issued_at` (the M2 re-issuance throttle)
/// before the client runs Apple's purchase sheet. If the user backs out of that
/// sheet, the reservation would otherwise linger for the throttle window —
/// making the reward look "scheduled" and 409-ing re-claims. The client calls
/// this on cancellation to clear the stamp so the reward is immediately
/// re-claimable.
///
/// Only clears while the reward is still `earned`; if a real redemption already
/// flipped it to delivered/delivering, this is a no-op. Idempotent, always 200.
pub async fn release_ios_offer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(reward_id): Path<i64>,
) -> Response {
    let caller = match state.auth.user_id(&headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let result = async {
        let mut tx = state.db.begin().await?;
        set_request_user(&mut tx, &caller).await?;
        sqlx::query(
            r#"
            UPDATE reward_ledger
            SET ios_offer_issued_at = NULL, updated_at = now()
            WHERE id = $1 AND user_id = $2 AND status = 'earned'
            "#,
        )
        .bind(reward_id)
        .bind(&caller)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok::<(), crate::AppError>(())
    }
    .await;

    match result {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "released" }))).into_response(),
        Err(e) => e.into_response(),
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
