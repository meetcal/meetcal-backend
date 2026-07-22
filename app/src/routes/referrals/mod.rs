//! Referral rewards program.
//!
//! Subscribers earn one free month (a 30-day reward unit) per 5 *qualified*
//! referrals. A referral qualifies when the referred user creates a NEW account
//! via a referral code, starts a paid (period_type == NORMAL, non-web) store
//! subscription, and survives 30 days with no refund / chargeback / fraud flag.
//!
//! Rewards are **derived**, never event-triggered: the qualification cron mints
//! `floor(qualified_count / 5)` reward rows minus those already minted, so a
//! disqualification that drops a referrer from 5 -> 4 can never double-mint when
//! they later re-cross 5. See [`rewards_to_mint`] / [`rewards_to_reverse`].
//!
//! Modules:
//!   * [`code`]            — invite-code generation + auto-create.
//!   * [`get_referral`]    — GET /users/me/referral.
//!   * [`redeem`]          — POST /users/me/referral/redeem.
//!   * [`webhook`]         — POST /webhooks/revenuecat.
//!   * [`run_qualification`] — POST /internal/referrals/run-qualification (cron).
//!   * [`claim`]           — POST /users/me/rewards/{id}/claim.
//!   * [`apple`] / [`play`] — platform delivery.

pub mod apple;
pub mod claim;
pub mod code;
pub mod get_referral;
pub mod play;
pub mod redeem;
pub mod run_qualification;
pub mod webhook;

use std::collections::HashMap;

/// One reward unit is 30 days.
pub const REWARD_DAYS: i64 = 30;
/// A qualified referral is worth 1/5 of a reward; 5 qualified => 1 reward.
pub const REFERRALS_PER_REWARD: i64 = 5;
/// Cap on reward months minted per user per calendar year.
pub const ANNUAL_REWARD_CAP: i64 = 12;
/// Qualification hold: a paid referral must survive this long to qualify.
pub const QUALIFICATION_HOLD_DAYS: i64 = 30;

/// Runtime configuration for the referral domain, read from the environment at
/// startup (mirrors `SlackConfig::from_env`). Held in `AppState`.
#[derive(Clone)]
pub struct ReferralConfig {
    /// Static bearer secret RevenueCat sends in the `Authorization` header.
    pub revenuecat_webhook_secret: String,
    /// Shared secret guarding the internal qualification-cron endpoint.
    pub internal_job_secret: String,
    /// Base for share URLs, e.g. `https://meetcal.app/invite`.
    pub share_base_url: String,
    /// Apple promotional-offer signing config; `None` when unconfigured.
    pub apple: Option<apple::AppleConfig>,
    /// Google Play delivery config; `None` when unconfigured.
    pub play: Option<play::PlayConfig>,
}

impl ReferralConfig {
    pub fn from_env() -> Self {
        Self {
            revenuecat_webhook_secret: env_str("REVENUECAT_WEBHOOK_SECRET"),
            internal_job_secret: env_str("INTERNAL_JOB_SECRET"),
            share_base_url: std::env::var("REFERRAL_SHARE_BASE_URL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "https://meetcal.app/invite".to_string()),
            apple: apple::AppleConfig::from_env(),
            play: play::PlayConfig::from_env(),
        }
    }

    pub fn share_url(&self, code: &str) -> String {
        format!("{}/{}", self.share_base_url.trim_end_matches('/'), code)
    }
}

fn env_str(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// Constant-time byte comparison for static secrets (webhook + internal-job).
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Parse a `key=value,key2=value2` env string into a map (used for per-product
/// Apple offer ids).
pub(crate) fn parse_kv_map(raw: &str) -> HashMap<String, String> {
    raw.split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let (k, v) = (k.trim(), v.trim());
            if k.is_empty() || v.is_empty() {
                None
            } else {
                Some((k.to_string(), v.to_string()))
            }
        })
        .collect()
}

// --- Pure state-machine logic (unit-tested) --------------------------------

/// Reward months a referrer *should* have, derived from their qualified count.
pub fn target_reward_count(qualified_count: i64) -> i64 {
    qualified_count / REFERRALS_PER_REWARD
}

/// How many NEW reward rows to mint.
///
/// * `qualified_count`  — current count of `qualified` referrals.
/// * `already_minted`   — reward rows already minted that are NOT reversed.
/// * `minted_this_year` — non-reversed reward rows earned in the current
///   calendar year (for the annual cap).
///
/// Returns `max(0, target - already_minted)` clamped to the remaining annual
/// cap. Because it compares against `already_minted` (an absolute count) rather
/// than "crossing a multiple of 5", a disqualification that reverses a reward
/// and later re-crosses 5 mints at most the deficit — never a double.
pub fn rewards_to_mint(qualified_count: i64, already_minted: i64, minted_this_year: i64) -> i64 {
    let target = target_reward_count(qualified_count);
    let deficit = (target - already_minted).max(0);
    let remaining_cap = (ANNUAL_REWARD_CAP - minted_this_year).max(0);
    deficit.min(remaining_cap)
}

/// How many `earned`-status reward rows to reverse (claw back) when a referrer's
/// qualified count drops below what they've been minted.
///
/// * `target`        — target reward count for the new (lower) qualified count.
/// * `active_total`  — all non-reversed reward rows.
/// * `active_earned` — non-reversed rows still in `earned` status (the only ones
///   eligible for clawback; claimed/delivered are kept).
///
/// Reverses only enough earned rows to bring the active total down to `target`.
pub fn rewards_to_reverse(target: i64, active_total: i64, active_earned: i64) -> i64 {
    let excess = (active_total - target).max(0);
    excess.min(active_earned)
}

/// Web-billing store? RevenueCat's web / RC Billing events are excluded from
/// referral qualification entirely.
pub fn is_web_store(store: &str) -> bool {
    matches!(
        store.trim().to_ascii_lowercase().as_str(),
        "rc_billing" | "web"
    )
}

/// Does this event mark a paid (NORMAL) subscription period? Trials and intro
/// offers do not count.
pub fn is_paid_conversion(event_type: &str, period_type: &str) -> bool {
    if !period_type.trim().eq_ignore_ascii_case("NORMAL") {
        return false;
    }
    matches!(
        event_type.trim().to_ascii_uppercase().as_str(),
        "INITIAL_PURCHASE"
            | "RENEWAL"
            | "NON_RENEWING_PURCHASE"
            | "PRODUCT_CHANGE"
            | "UNCANCELLATION"
    )
}

/// Does this event look like the referrer's promotional free month being
/// redeemed (iOS delivery confirmation)?
///
/// Discriminator: a renewal-type event (`RENEWAL` / `PRODUCT_CHANGE`) with
/// `period_type == NORMAL` that cost nothing (`price == 0`). RevenueCat's webhook
/// does not consistently surface a distinct promotional-offer identifier for
/// StoreKit promotional offers, so a $0 paid renewal is the most robust signal
/// the payload reliably supports. ASSUMPTION to sandbox-validate in Phase 3:
/// the promo month arrives as a $0 NORMAL renewal (not a $0 TRIAL/INTRO).
pub fn is_promo_redemption(event_type: &str, period_type: &str, price: Option<f64>) -> bool {
    let renewal = matches!(
        event_type.trim().to_ascii_uppercase().as_str(),
        "RENEWAL" | "PRODUCT_CHANGE"
    );
    renewal && period_type.trim().eq_ignore_ascii_case("NORMAL") && price == Some(0.0)
}

/// Does this event disqualify a referral (refund / chargeback / fraud)? Note a
/// plain CANCELLATION (auto-renew off) does NOT disqualify.
pub fn is_disqualifying(event_type: &str) -> bool {
    matches!(
        event_type.trim().to_ascii_uppercase().as_str(),
        "REFUND" | "CHARGEBACK" | "SUBSCRIPTION_PAUSED_FRAUD" | "BILLING_FRAUD"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_from_qualified_count() {
        assert_eq!(target_reward_count(0), 0);
        assert_eq!(target_reward_count(4), 0);
        assert_eq!(target_reward_count(5), 1);
        assert_eq!(target_reward_count(9), 1);
        assert_eq!(target_reward_count(10), 2);
    }

    #[test]
    fn mint_basic() {
        // 5 qualified, none minted yet -> mint 1.
        assert_eq!(rewards_to_mint(5, 0, 0), 1);
        // 10 qualified, 1 minted -> mint 1 more.
        assert_eq!(rewards_to_mint(10, 1, 1), 1);
        // 9 qualified, 1 minted -> nothing new.
        assert_eq!(rewards_to_mint(9, 1, 1), 0);
    }

    #[test]
    fn disqualify_after_qualify_never_double_mints() {
        // Referrer hit 5 qualified and minted 1 reward (still counts as minted).
        // A disqualification drops them to 4; the earned reward is reversed
        // elsewhere. If they later re-cross to 5, `already_minted` reflects the
        // reversal (0), so exactly one reward is minted — not two.
        //
        // Scenario A: reward was reversed, already_minted drops to 0.
        assert_eq!(rewards_to_mint(5, 0, 0), 1); // re-cross mints 1 (correct)
        // Scenario B: reward was DELIVERED (kept), already_minted stays 1.
        // Re-crossing 4 -> 5 must NOT mint again.
        assert_eq!(rewards_to_mint(5, 1, 1), 0);
    }

    #[test]
    fn annual_cap_enforced() {
        // 100 qualified would target 20 rewards, but the cap is 12/year.
        assert_eq!(rewards_to_mint(100, 0, 0), ANNUAL_REWARD_CAP);
        // 11 already earned this year -> only 1 more allowed.
        assert_eq!(rewards_to_mint(100, 11, 11), 1);
        // Cap already reached this year.
        assert_eq!(rewards_to_mint(100, 12, 12), 0);
    }

    #[test]
    fn reverse_only_earned_down_to_target() {
        // Active total 1 (all earned), target now 0 -> reverse 1.
        assert_eq!(rewards_to_reverse(0, 1, 1), 1);
        // Active total 1 but it's delivered (0 earned), target 0 -> reverse none.
        assert_eq!(rewards_to_reverse(0, 1, 0), 0);
        // Active total 3 (2 earned, 1 delivered), target 1 -> reverse 2 excess,
        // but only 2 are earned so reverse 2.
        assert_eq!(rewards_to_reverse(1, 3, 2), 2);
        // Target still satisfied -> reverse none.
        assert_eq!(rewards_to_reverse(2, 2, 2), 0);
    }

    #[test]
    fn web_store_excluded() {
        assert!(is_web_store("rc_billing"));
        assert!(is_web_store("RC_BILLING"));
        assert!(is_web_store("web"));
        assert!(!is_web_store("APP_STORE"));
        assert!(!is_web_store("PLAY_STORE"));
        assert!(!is_web_store("stripe"));
    }

    #[test]
    fn paid_conversion_requires_normal_period() {
        assert!(is_paid_conversion("INITIAL_PURCHASE", "NORMAL"));
        assert!(is_paid_conversion("RENEWAL", "normal"));
        // Trials / intro do not count.
        assert!(!is_paid_conversion("INITIAL_PURCHASE", "TRIAL"));
        assert!(!is_paid_conversion("INITIAL_PURCHASE", "INTRO"));
        // Non-purchase events don't convert.
        assert!(!is_paid_conversion("CANCELLATION", "NORMAL"));
    }

    #[test]
    fn disqualifying_events() {
        assert!(is_disqualifying("REFUND"));
        assert!(is_disqualifying("chargeback"));
        // Auto-renew off is NOT disqualifying.
        assert!(!is_disqualifying("CANCELLATION"));
        assert!(!is_disqualifying("RENEWAL"));
    }

    #[test]
    fn promo_redemption_discriminator() {
        // $0 NORMAL renewal = promo month applied.
        assert!(is_promo_redemption("RENEWAL", "NORMAL", Some(0.0)));
        assert!(is_promo_redemption("PRODUCT_CHANGE", "normal", Some(0.0)));
        // A paid renewal is not a promo redemption.
        assert!(!is_promo_redemption("RENEWAL", "NORMAL", Some(9.99)));
        // Unknown price is not a redemption (avoid false positives).
        assert!(!is_promo_redemption("RENEWAL", "NORMAL", None));
        // Non-renewal / non-normal.
        assert!(!is_promo_redemption(
            "INITIAL_PURCHASE",
            "NORMAL",
            Some(0.0)
        ));
        assert!(!is_promo_redemption("RENEWAL", "TRIAL", Some(0.0)));
    }

    #[test]
    fn kv_map_parsing() {
        let m = parse_kv_map("com.app.monthly=off_monthly,com.app.annual=off_annual");
        assert_eq!(m.get("com.app.monthly").unwrap(), "off_monthly");
        assert_eq!(m.get("com.app.annual").unwrap(), "off_annual");
        assert!(parse_kv_map("").is_empty());
        assert!(parse_kv_map("garbage").is_empty());
    }
}
