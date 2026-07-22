//! POST /webhooks/revenuecat — ingest RevenueCat subscription events.
//!
//! Auth: RevenueCat sends a static value in the `Authorization` header that we
//! compare (constant-time) against `REVENUECAT_WEBHOOK_SECRET`.
//!
//! Flow: audit-log every event idempotently (UNIQUE event_id), then drive the
//! referral state machine — a first paid (NORMAL, non-web) period promotes a
//! pending referral to `qualifying`; a refund/chargeback disqualifies it and
//! reverses any still-`earned` rewards of the referrer.

use crate::{
    AppState,
    routes::referrals::{
        ct_eq, is_disqualifying, is_paid_conversion, is_promo_redemption, is_web_store,
        rewards_to_reverse,
    },
    routes::users::auth::set_backend_context,
};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::Value;

fn authorized(secret: &str, headers: &HeaderMap) -> bool {
    if secret.is_empty() {
        return false;
    }
    let Some(provided) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    // Accept either the raw secret or a `Bearer <secret>` form.
    let provided = provided.strip_prefix("Bearer ").unwrap_or(provided);
    ct_eq(provided.as_bytes(), secret.as_bytes())
}

fn field<'a>(event: &'a Value, key: &str) -> &'a str {
    event.get(key).and_then(Value::as_str).unwrap_or("")
}

pub async fn revenuecat_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let cfg = &state.referrals;

    if cfg.revenuecat_webhook_secret.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "RevenueCat webhook is not configured",
        )
            .into_response();
    }
    if !authorized(&cfg.revenuecat_webhook_secret, &headers) {
        return (StatusCode::UNAUTHORIZED, "bad webhook auth").into_response();
    }

    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "invalid JSON").into_response();
    };
    // RevenueCat wraps the event: { "event": {...}, "api_version": "1.0" }.
    let event = payload.get("event").unwrap_or(&payload).clone();
    let event_id = field(&event, "id");
    if event_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing event id").into_response();
    }

    match handle(&state, &event, event_id).await {
        Ok(()) => (StatusCode::OK, "ok").into_response(),
        Err(e) => e.into_response(),
    }
}

async fn handle(state: &AppState, event: &Value, event_id: &str) -> Result<(), crate::AppError> {
    let app_user_id = field(event, "app_user_id");
    let store = field(event, "store");
    let event_type = field(event, "type");
    let period_type = field(event, "period_type");
    let product_id = field(event, "product_id");

    let mut tx = state.db.begin().await?;
    set_backend_context(&mut tx).await?;

    // 1. Idempotent audit log. If we've seen this event_id, stop (return 200).
    let inserted = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO subscription_events
            (event_id, app_user_id, store, type, period_type, product_id, event)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (event_id) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(event_id)
    .bind(opt(app_user_id))
    .bind(opt(store))
    .bind(opt(event_type))
    .bind(opt(period_type))
    .bind(opt(product_id))
    .bind(event)
    .fetch_optional(&mut *tx)
    .await?;

    if inserted.is_none() {
        // Duplicate delivery — already processed.
        tx.commit().await?;
        return Ok(());
    }

    // 2. Web-billing events are excluded from qualification entirely.
    if is_web_store(store) || app_user_id.is_empty() {
        tx.commit().await?;
        return Ok(());
    }

    // 3. First paid (NORMAL) period -> promote a pending referral to qualifying.
    //    Anchor the 30-day hold to the event's purchase time (fallback the event
    //    timestamp, then now()) so a delayed / retried webhook does not shift it.
    if is_paid_conversion(event_type, period_type) {
        let started_at = event_millis(event, &["purchased_at_ms", "event_timestamp_ms"])
            .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis);
        sqlx::query(
            r#"
            UPDATE referrals
            SET status = 'qualifying',
                subscription_started_at = COALESCE(subscription_started_at, $2, now()),
                updated_at = now()
            WHERE referred_user_id = $1
              AND status = 'pending'
            "#,
        )
        .bind(app_user_id)
        .bind(started_at)
        .execute(&mut *tx)
        .await?;
    }

    // 3b. Promotional free month applied -> confirm delivery of the OLDEST earned
    //     reward for this user (iOS delivery confirmation for the C1 redesign).
    //     Idempotent: the event_id dedup above guarantees this body runs once per
    //     unique event, so exactly one reward is delivered per redemption and a
    //     duplicate event delivers none.
    let price = event_f64(event, &["price_in_purchased_currency", "price"]);
    if is_promo_redemption(event_type, period_type, price) {
        deliver_ios_reward_for(&mut tx, app_user_id, event_id).await?;
    }

    // 4. Refund / chargeback / fraud -> disqualify + reverse earned rewards.
    if is_disqualifying(event_type) {
        let referral = sqlx::query_as::<_, (String, String)>(
            "SELECT referrer_user_id, status FROM referrals WHERE referred_user_id = $1",
        )
        .bind(app_user_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some((referrer, status)) = referral
            && (status == "pending" || status == "qualifying" || status == "qualified")
        {
            sqlx::query(
                r#"
                UPDATE referrals
                SET status = 'disqualified',
                    disqualification_reason = $2,
                    updated_at = now()
                WHERE referred_user_id = $1
                "#,
            )
            .bind(app_user_id)
            .bind(event_type)
            .execute(&mut *tx)
            .await?;

            // Only a previously-qualified referral could have contributed to a
            // mint, so recompute reversals for the referrer.
            if status == "qualified" {
                reverse_rewards_for(&mut tx, &referrer).await?;
            }
        }
    }

    tx.commit().await?;
    Ok(())
}

/// Reverse just enough `earned`-status rewards to bring the referrer's active
/// reward count back down to their (now lower) target. Claimed/delivered rewards
/// are never clawed back.
async fn reverse_rewards_for(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    referrer: &str,
) -> Result<(), crate::AppError> {
    let qualified_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM referrals WHERE referrer_user_id = $1 AND status = 'qualified'",
    )
    .bind(referrer)
    .fetch_one(&mut **tx)
    .await?;

    let active_total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM reward_ledger WHERE user_id = $1 AND status <> 'reversed'",
    )
    .bind(referrer)
    .fetch_one(&mut **tx)
    .await?;

    let active_earned = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM reward_ledger WHERE user_id = $1 AND status = 'earned'",
    )
    .bind(referrer)
    .fetch_one(&mut **tx)
    .await?;

    let target = crate::routes::referrals::target_reward_count(qualified_count);
    let to_reverse = rewards_to_reverse(target, active_total, active_earned);

    if to_reverse > 0 {
        sqlx::query(
            r#"
            UPDATE reward_ledger
            SET status = 'reversed', reversed_at = now(), updated_at = now()
            WHERE id IN (
                SELECT id FROM reward_ledger
                WHERE user_id = $1 AND status = 'earned'
                ORDER BY earned_at ASC, id ASC
                LIMIT $2
            )
            "#,
        )
        .bind(referrer)
        .bind(to_reverse)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

/// Confirm delivery of the oldest still-`earned` reward for `app_user_id`,
/// tagging it with the redemption `event_id`. `FOR UPDATE SKIP LOCKED` keeps two
/// concurrent (distinct) redemption events from both claiming the same row.
async fn deliver_ios_reward_for(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_user_id: &str,
    event_id: &str,
) -> Result<(), crate::AppError> {
    sqlx::query(
        r#"
        UPDATE reward_ledger
        SET status = 'delivered',
            platform = COALESCE(platform, 'ios'),
            store_transaction_id = $2,
            delivered_at = now(),
            updated_at = now()
        WHERE id = (
            SELECT id FROM reward_ledger
            WHERE user_id = $1 AND status = 'earned'
            ORDER BY earned_at ASC, id ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        "#,
    )
    .bind(app_user_id)
    .bind(event_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn opt(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

/// First present integer-valued millis field among `keys` (accepts number or
/// numeric string).
fn event_millis(event: &Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(v) = event.get(*key) {
            if let Some(n) = v.as_i64() {
                return Some(n);
            }
            if let Some(n) = v.as_str().and_then(|s| s.parse::<i64>().ok()) {
                return Some(n);
            }
        }
    }
    None
}

/// First present numeric field among `keys` (accepts number or numeric string).
fn event_f64(event: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(v) = event.get(*key) {
            if let Some(n) = v.as_f64() {
                return Some(n);
            }
            if let Some(n) = v.as_str().and_then(|s| s.parse::<f64>().ok()) {
                return Some(n);
            }
        }
    }
    None
}
