//! Best-effort RevenueCat REST screening for the redeem first-time-paid check.
//!
//! At launch the local `subscription_events` table is empty, so a purely-local
//! "has this user ever paid?" check is blind — two existing paying subscribers
//! could redeem each other's codes. When `REVENUECAT_SECRET_API_KEY` is set, we
//! additionally ask RevenueCat whether the caller already has non-web paid
//! (non-trial) history and reject the redeem if so.
//!
//! This is ADDITIVE screening: it fails OPEN (logs and returns `Ok(false)`) on
//! any transport/parse error — availability over strictness — and never 503s.
//! Follows the outbound-HTTP pattern in `play.rs`.

use serde_json::Value;
use std::time::Duration;

const SUBSCRIBER_URL_BASE: &str = "https://api.revenuecat.com/v1/subscribers/";
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct RevenueCatApi {
    api_key: String,
    http: reqwest::Client,
}

impl RevenueCatApi {
    /// Construct a client when an API key is configured, else `None`.
    pub fn from_key(api_key: &str) -> Option<Self> {
        if api_key.trim().is_empty() {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .ok()?;
        Some(Self {
            api_key: api_key.to_string(),
            http,
        })
    }

    /// True if the subscriber already has any non-web paid (non-trial)
    /// subscription history. Fails OPEN: logs and returns `Ok(false)` on error so
    /// a RevenueCat outage never blocks a legitimate redeem.
    pub async fn has_non_web_paid_history(&self, app_user_id: &str) -> bool {
        match self.fetch(app_user_id).await {
            Ok(subscriber) => subscriber_has_paid_history(&subscriber),
            Err(e) => {
                eprintln!(
                    "referral: RevenueCat subscriber lookup failed (failing open) for \
                     app_user_id={app_user_id}: {e}"
                );
                false
            }
        }
    }

    async fn fetch(&self, app_user_id: &str) -> Result<Value, String> {
        // Path-encode the id so reserved characters can't break out of the path.
        let url = format!(
            "{SUBSCRIBER_URL_BASE}{}",
            crate::routes::referrals::play::encode_segment(app_user_id)
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("status {}", resp.status()));
        }
        resp.json::<Value>().await.map_err(|e| e.to_string())
    }
}

/// Inspect a RevenueCat subscriber payload for any non-web paid (non-trial)
/// subscription. Pure, for testing.
pub fn subscriber_has_paid_history(body: &Value) -> bool {
    let Some(subs) = body
        .get("subscriber")
        .and_then(|s| s.get("subscriptions"))
        .and_then(Value::as_object)
    else {
        return false;
    };

    subs.values().any(|sub| {
        let store = sub
            .get("store")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let period = sub
            .get("period_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let non_web = !matches!(store.as_str(), "web" | "rc_billing" | "stripe" | "");
        let paid = period == "normal";
        non_web && paid
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_paid_app_store_history() {
        let body = json!({
            "subscriber": {
                "subscriptions": {
                    "com.meetcal.monthly": { "store": "app_store", "period_type": "normal" }
                }
            }
        });
        assert!(subscriber_has_paid_history(&body));
    }

    #[test]
    fn trial_only_is_not_paid_history() {
        let body = json!({
            "subscriber": {
                "subscriptions": {
                    "com.meetcal.monthly": { "store": "play_store", "period_type": "trial" }
                }
            }
        });
        assert!(!subscriber_has_paid_history(&body));
    }

    #[test]
    fn web_billing_is_excluded() {
        let body = json!({
            "subscriber": {
                "subscriptions": {
                    "com.meetcal.monthly": { "store": "rc_billing", "period_type": "normal" }
                }
            }
        });
        assert!(!subscriber_has_paid_history(&body));
    }

    #[test]
    fn no_subscriptions_is_not_paid() {
        assert!(!subscriber_has_paid_history(
            &json!({ "subscriber": { "subscriptions": {} } })
        ));
        assert!(!subscriber_has_paid_history(&json!({})));
    }
}
