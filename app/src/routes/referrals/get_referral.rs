//! GET /users/me/referral — the caller's referral dashboard.

use crate::{
    AppError, AppState, routes::referrals::code::ensure_referral_code,
    routes::users::auth::set_request_user,
};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RewardSummary {
    pub id: i64,
    pub status: String,
    pub earned_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ReferralResponse {
    pub code: String,
    pub share_url: String,
    /// Referred users who signed up but have not paid yet.
    pub pending_count: i64,
    /// Referred users who paid and are inside the 30-day hold.
    pub qualifying_count: i64,
    /// Referred users who fully qualified.
    pub qualified_count: i64,
    /// Reward months currently claimable (rewards in `earned` status).
    pub reward_months_available: i64,
    pub rewards: Vec<RewardSummary>,
}

/// /users/me/referral endpoint
///
/// curl 'https://api.meetcal.app/users/me/referral' \
///   -H 'Authorization: Bearer <clerk-jwt>' | jq .
///
/// Returns the caller's referral code (auto-created on first call), a share URL,
/// anonymized progress counts, and their reward ledger. The referred users'
/// identities are never exposed — only counts.
///
/// {
///   "code": "AB2C4D6E",
///   "share_url": "https://meetcal.app/invite/AB2C4D6E",
///   "pending_count": 2,
///   "qualifying_count": 1,
///   "qualified_count": 5,
///   "reward_months_available": 1,
///   "rewards": [{ "id": 12, "status": "earned", "earned_at": "2026-07-01T00:00:00Z" }]
/// }
pub async fn get_referral(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ReferralResponse>, AppError> {
    let user_id = state.auth.user_id(&headers).await?;
    let mut tx = state.db.begin().await?;
    set_request_user(&mut tx, &user_id).await?;

    let code = ensure_referral_code(&mut tx, &user_id).await?;

    // Anonymized progress: counts per status, caller as referrer.
    let (pending_count, qualifying_count, qualified_count): (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE status = 'pending')::BIGINT,
            COUNT(*) FILTER (WHERE status = 'qualifying')::BIGINT,
            COUNT(*) FILTER (WHERE status = 'qualified')::BIGINT
        FROM referrals
        WHERE referrer_user_id = $1
        "#,
    )
    .bind(&user_id)
    .fetch_one(&mut *tx)
    .await?;

    let rewards = sqlx::query_as::<_, (i64, String, DateTime<Utc>)>(
        r#"
        SELECT id, status, earned_at
        FROM reward_ledger
        WHERE user_id = $1
        ORDER BY earned_at DESC, id DESC
        "#,
    )
    .bind(&user_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;

    let reward_months_available = rewards
        .iter()
        .filter(|(_, status, _)| status == "earned")
        .count() as i64;

    let rewards = rewards
        .into_iter()
        .map(|(id, status, earned_at)| RewardSummary {
            id,
            status,
            earned_at,
        })
        .collect();

    Ok(Json(ReferralResponse {
        code: code.clone(),
        share_url: state.referrals.share_url(&code),
        pending_count,
        qualifying_count,
        qualified_count,
        reward_months_available,
        rewards,
    }))
}
