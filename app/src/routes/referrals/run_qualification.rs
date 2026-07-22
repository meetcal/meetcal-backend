//! POST /internal/referrals/run-qualification — the daily qualification job.
//!
//! Guarded by the `X-Internal-Job-Secret` header (compared constant-time to
//! `INTERNAL_JOB_SECRET`). Not exposed through CORS; called only by the OS cron
//! script `app/deploy/run-qualification.sh`.
//!
//! Two phases, both idempotent:
//!   1. Promote `qualifying` -> `qualified` where the 30-day hold has cleared and
//!      no disqualifying event exists.
//!   2. Mint reward rows per the derived formula (see [`rewards_to_mint`]),
//!      respecting the 12-per-calendar-year cap. Deterministic idempotency keys
//!      make re-runs and concurrent runs safe.

use crate::{
    AppState,
    routes::referrals::{QUALIFICATION_HOLD_DAYS, ct_eq, rewards_to_mint, target_reward_count},
    routes::users::auth::set_backend_context,
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RunQualificationResponse {
    pub promoted: i64,
    pub minted: i64,
}

const SECRET_HEADER: &str = "x-internal-job-secret";

pub async fn run_qualification(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let cfg = &state.referrals;
    if cfg.internal_job_secret.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "internal job endpoint is not configured",
        )
            .into_response();
    }
    let provided = headers
        .get(SECRET_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ct_eq(provided.as_bytes(), cfg.internal_job_secret.as_bytes()) {
        return (StatusCode::UNAUTHORIZED, "bad job secret").into_response();
    }

    match run(&state).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn run(state: &AppState) -> Result<RunQualificationResponse, crate::AppError> {
    let mut tx = state.db.begin().await?;
    set_backend_context(&mut tx).await?;

    // Phase 1: promote matured qualifying referrals. A disqualifying event would
    // already have moved the row to `disqualified`, but we also guard explicitly
    // against any disqualifying subscription_event for the referred user.
    let hold = format!("{QUALIFICATION_HOLD_DAYS} days");
    let promoted = sqlx::query_scalar::<_, i64>(
        r#"
        WITH promoted AS (
            UPDATE referrals r
            SET status = 'qualified', qualified_at = now(), updated_at = now()
            WHERE r.status = 'qualifying'
              AND r.subscription_started_at IS NOT NULL
              AND r.subscription_started_at <= now() - $1::interval
              AND NOT EXISTS (
                  SELECT 1 FROM subscription_events e
                  WHERE e.app_user_id = r.referred_user_id
                    AND upper(coalesce(e.type, '')) IN ('REFUND', 'CHARGEBACK', 'BILLING_FRAUD')
              )
            RETURNING 1
        )
        SELECT COUNT(*)::BIGINT FROM promoted
        "#,
    )
    .bind(&hold)
    .fetch_one(&mut *tx)
    .await?;

    // Phase 2: mint. Process every referrer with at least one qualified referral.
    let referrers = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT referrer_user_id FROM referrals WHERE status = 'qualified'",
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut minted_total = 0i64;
    for referrer in referrers {
        minted_total += mint_for(&mut tx, &referrer).await?;
    }

    tx.commit().await?;

    Ok(RunQualificationResponse {
        promoted,
        minted: minted_total,
    })
}

/// Mint any owed rewards for one referrer. Returns how many rows were minted.
async fn mint_for(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    referrer: &str,
) -> Result<i64, crate::AppError> {
    let qualified_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM referrals WHERE referrer_user_id = $1 AND status = 'qualified'",
    )
    .bind(referrer)
    .fetch_one(&mut **tx)
    .await?;

    // Non-reversed rewards count as already minted (reversed ones freed their slot).
    let already_minted = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM reward_ledger WHERE user_id = $1 AND status <> 'reversed'",
    )
    .bind(referrer)
    .fetch_one(&mut **tx)
    .await?;

    // Annual cap counts non-reversed rewards earned this calendar year. Pin the
    // year boundary to UTC so it doesn't drift with the session/server timezone.
    let minted_this_year = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::BIGINT FROM reward_ledger
        WHERE user_id = $1
          AND status <> 'reversed'
          AND date_part('year', earned_at AT TIME ZONE 'UTC')
              = date_part('year', now() AT TIME ZONE 'UTC')
        "#,
    )
    .bind(referrer)
    .fetch_one(&mut **tx)
    .await?;

    let to_mint = rewards_to_mint(qualified_count, already_minted, minted_this_year);
    if to_mint == 0 {
        return Ok(0);
    }
    // Log the target for observability of the derived formula.
    let _ = target_reward_count(qualified_count);

    // Idempotency keys must never be reused, even across reversals, so we base
    // the ordinal on ALL rows ever minted for this user (including reversed),
    // not just the active ones.
    let total_ever = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM reward_ledger WHERE user_id = $1",
    )
    .bind(referrer)
    .fetch_one(&mut **tx)
    .await?;

    let mut minted = 0i64;
    for i in 0..to_mint {
        let ordinal = total_ever + 1 + i;
        let key = format!("mint:{referrer}:{ordinal}");
        let inserted = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO reward_ledger (user_id, status, idempotency_key)
            VALUES ($1, 'earned', $2)
            ON CONFLICT (idempotency_key) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(referrer)
        .bind(&key)
        .fetch_optional(&mut **tx)
        .await?;
        if inserted.is_some() {
            minted += 1;
        }
    }

    Ok(minted)
}
