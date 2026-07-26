//! POST /users/me/referral/redeem — attribute the caller to a referrer.

use crate::{
    AppError, AppState,
    routes::users::auth::{set_backend_context, set_request_user},
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct RedeemRequest {
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct RedeemResponse {
    pub status: String,
}

/// Why a redeem attempt was rejected. The `code()` is the stable machine-readable
/// error string clients switch on.
#[derive(Debug, PartialEq, Eq)]
pub enum RedeemError {
    /// The code belongs to the caller.
    SelfReferral,
    /// The caller is already attributed to a referrer.
    AlreadyRedeemed,
    /// No such code.
    InvalidCode,
    /// The caller already has/had a paid subscription — attribution must happen
    /// before any paid subscription exists.
    NotEligible,
}

impl RedeemError {
    pub fn code(&self) -> &'static str {
        match self {
            RedeemError::SelfReferral => "self_referral",
            RedeemError::AlreadyRedeemed => "already_redeemed",
            RedeemError::InvalidCode => "invalid_code",
            RedeemError::NotEligible => "not_eligible",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            RedeemError::InvalidCode => StatusCode::NOT_FOUND,
            RedeemError::AlreadyRedeemed => StatusCode::CONFLICT,
            RedeemError::SelfReferral | RedeemError::NotEligible => StatusCode::BAD_REQUEST,
        }
    }
}

/// Pure redemption policy. Returns the referrer's user id on success.
///
/// * `caller`          — the redeeming (referred) user.
/// * `code_owner`      — owner of the submitted code, if it exists.
/// * `already_referred`— caller already attributed to someone.
/// * `has_prior_paid`  — caller already has/had a non-web paid subscription.
pub fn validate_redeem(
    caller: &str,
    code_owner: Option<&str>,
    already_referred: bool,
    has_prior_paid: bool,
) -> Result<String, RedeemError> {
    let owner = code_owner.ok_or(RedeemError::InvalidCode)?;
    if owner == caller {
        return Err(RedeemError::SelfReferral);
    }
    if already_referred {
        return Err(RedeemError::AlreadyRedeemed);
    }
    if has_prior_paid {
        return Err(RedeemError::NotEligible);
    }
    Ok(owner.to_string())
}

/// /users/me/referral/redeem endpoint
///
/// curl -X POST 'https://api.meetcal.app/users/me/referral/redeem' \
///   -H 'Authorization: Bearer <clerk-jwt>' \
///   -H 'Content-Type: application/json' \
///   -d '{"code":"AB2C4D6E"}' | jq .
///
/// Attributes the CALLER as the referred user for the code's owner. On success:
///   { "status": "pending" }
/// On rejection, HTTP 400/404/409 with { "error": "<code>" } where <code> is one
/// of: self_referral, already_redeemed, invalid_code, not_eligible.
pub async fn redeem(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RedeemRequest>,
) -> Response {
    match redeem_inner(&state, &headers, &body).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(RedeemOutcome::Rejected(err)) => {
            (err.status(), Json(json!({ "error": err.code() }))).into_response()
        }
        Err(RedeemOutcome::Internal(err)) => err.into_response(),
    }
}

enum RedeemOutcome {
    Rejected(RedeemError),
    Internal(AppError),
}

impl From<AppError> for RedeemOutcome {
    fn from(e: AppError) -> Self {
        RedeemOutcome::Internal(e)
    }
}

impl From<sqlx::Error> for RedeemOutcome {
    fn from(e: sqlx::Error) -> Self {
        RedeemOutcome::Internal(AppError::from(e))
    }
}

async fn redeem_inner(
    state: &AppState,
    headers: &HeaderMap,
    body: &RedeemRequest,
) -> Result<RedeemResponse, RedeemOutcome> {
    let caller = state.auth.user_id(headers).await?;
    let code = body.code.trim();
    if code.is_empty() {
        return Err(RedeemOutcome::Rejected(RedeemError::InvalidCode));
    }

    let mut tx = state.db.begin().await?;
    // Redeem is server-authoritative: it must resolve another user's code and
    // read the backend-only subscription_events table.
    set_request_user(&mut tx, &caller).await?;
    set_backend_context(&mut tx).await?;

    let code_owner =
        sqlx::query_scalar::<_, String>("SELECT user_id FROM referral_codes WHERE code = $1")
            .bind(code)
            .fetch_optional(&mut *tx)
            .await?;

    let already_referred =
        sqlx::query_scalar::<_, i32>("SELECT 1 FROM referrals WHERE referred_user_id = $1")
            .bind(&caller)
            .fetch_optional(&mut *tx)
            .await?
            .is_some();

    // Any prior non-web paid (NORMAL) event makes the caller ineligible. Local
    // events only — blind before the webhook has any history, hence the remote
    // screening below.
    let local_prior_paid = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT 1 FROM subscription_events
        WHERE app_user_id = $1
          AND upper(coalesce(period_type, '')) = 'NORMAL'
          AND lower(coalesce(store, '')) NOT IN ('web', 'rc_billing')
        LIMIT 1
        "#,
    )
    .bind(&caller)
    .fetch_optional(&mut *tx)
    .await?
    .is_some();

    // Best-effort remote screening: if a RevenueCat REST key is configured, ask
    // whether the caller already has non-web paid history. Additive and fails
    // OPEN (see RevenueCatApi) so a RevenueCat outage never blocks a redeem.
    let remote_prior_paid = if !local_prior_paid {
        match crate::routes::referrals::revenuecat_api::RevenueCatApi::from_key(
            &state.referrals.revenuecat_secret_api_key,
        ) {
            Some(api) => api.has_non_web_paid_history(&caller).await,
            None => false,
        }
    } else {
        false
    };

    let referrer = validate_redeem(
        &caller,
        code_owner.as_deref(),
        already_referred,
        local_prior_paid || remote_prior_paid,
    )
    .map_err(RedeemOutcome::Rejected)?;

    // ON CONFLICT guards the race where two requests attribute the same caller.
    let inserted = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO referrals (referrer_user_id, referred_user_id, referral_code, status)
        VALUES ($1, $2, $3, 'pending')
        ON CONFLICT (referred_user_id) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(&referrer)
    .bind(&caller)
    .bind(code)
    .fetch_optional(&mut *tx)
    .await?;

    if inserted.is_none() {
        return Err(RedeemOutcome::Rejected(RedeemError::AlreadyRedeemed));
    }

    tx.commit().await?;

    Ok(RedeemResponse {
        status: "pending".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_code() {
        assert_eq!(
            validate_redeem("user_a", None, false, false),
            Err(RedeemError::InvalidCode)
        );
    }

    #[test]
    fn rejects_self_referral() {
        assert_eq!(
            validate_redeem("user_a", Some("user_a"), false, false),
            Err(RedeemError::SelfReferral)
        );
    }

    #[test]
    fn rejects_double_redeem() {
        assert_eq!(
            validate_redeem("user_a", Some("user_b"), true, false),
            Err(RedeemError::AlreadyRedeemed)
        );
    }

    #[test]
    fn rejects_prior_paid() {
        assert_eq!(
            validate_redeem("user_a", Some("user_b"), false, true),
            Err(RedeemError::NotEligible)
        );
    }

    #[test]
    fn accepts_valid_redeem() {
        assert_eq!(
            validate_redeem("user_a", Some("user_b"), false, false),
            Ok("user_b".to_string())
        );
    }

    #[test]
    fn self_referral_takes_precedence_over_other_flags() {
        // Own code is checked before already-redeemed / eligibility.
        assert_eq!(
            validate_redeem("user_a", Some("user_a"), true, true),
            Err(RedeemError::SelfReferral)
        );
    }
}
