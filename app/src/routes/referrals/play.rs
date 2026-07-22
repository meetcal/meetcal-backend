//! Google Play delivery for reward months (Android).
//!
//! A reward month on Android is delivered by DEFERRING the subscriber's next
//! renewal by 30 days via the Google Play Developer API
//! `purchases.subscriptions.defer` (androidpublisher v3).
//!
//! !!! LEGACY API — FLAGGED FOR VALIDATION !!!
//! `purchases.subscriptions.defer` belongs to Google's legacy Play Developer
//! subscriptions API, which Google has deprecated in favour of the Subscriptions
//! v2 / Monetization model. Before shipping Android delivery (Phase 2), CONFIRM
//! this endpoint is still available for our integration and identify the fallback
//! (Play promotional offers with a claim flow, mirroring iOS) if it is not. A
//! RevenueCat "granted entitlement" is NOT a fallback: it grants access but does
//! not stop Google from charging.

use serde::Deserialize;

/// The seam between the claim handler and Google's API, so delivery can be
/// stubbed in tests and swapped for the v2 fallback later. Used only via static
/// dispatch, so the async-fn-in-trait auto-bound caveat does not apply.
#[allow(async_fn_in_trait)]
pub trait PlayBilling {
    /// Defer the subscription's renewal, extending expiry by
    /// `REWARD_DAYS`. Must be safe to call at most once per reward (the claim
    /// handler enforces idempotency via status guards + idempotency_key).
    async fn defer(&self, req: &DeferRequest) -> Result<DeferOutcome, PlayError>;
}

#[derive(Debug, Clone)]
pub struct DeferRequest {
    pub package_name: String,
    pub subscription_id: String,
    pub purchase_token: String,
    /// Current expiry in epoch millis (Google requires the expected expiry).
    pub current_expiry_ms: i64,
}

#[derive(Debug)]
pub struct DeferOutcome {
    pub new_expiry_ms: i64,
}

#[derive(Debug)]
pub enum PlayError {
    Auth(String),
    Api(String),
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayError::Auth(m) => write!(f, "play auth error: {m}"),
            PlayError::Api(m) => write!(f, "play api error: {m}"),
        }
    }
}

#[derive(Clone)]
pub struct PlayConfig {
    pub package_name: String,
    /// Raw service-account JSON key.
    service_account_json: String,
}

impl PlayConfig {
    pub fn from_env() -> Option<Self> {
        let package_name = std::env::var("GOOGLE_PLAY_PACKAGE_NAME").unwrap_or_default();
        let service_account_json =
            std::env::var("GOOGLE_PLAY_SERVICE_ACCOUNT_JSON").unwrap_or_default();
        if package_name.is_empty() || service_account_json.trim().is_empty() {
            return None;
        }
        Some(Self {
            package_name,
            service_account_json,
        })
    }

    pub fn billing(&self) -> Result<GooglePlayBilling, PlayError> {
        let creds: ServiceAccount = serde_json::from_str(&self.service_account_json)
            .map_err(|e| PlayError::Auth(format!("bad service account json: {e}")))?;
        Ok(GooglePlayBilling {
            creds,
            http: reqwest::Client::new(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct ServiceAccount {
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

/// Real Google Play delivery via androidpublisher v3.
pub struct GooglePlayBilling {
    creds: ServiceAccount,
    http: reqwest::Client,
}

impl GooglePlayBilling {
    /// Mint a short-lived OAuth2 access token via the service-account JWT grant.
    async fn access_token(&self) -> Result<String, PlayError> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

        #[derive(serde::Serialize)]
        struct Claims<'a> {
            iss: &'a str,
            scope: &'a str,
            aud: &'a str,
            iat: i64,
            exp: i64,
        }

        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            iss: &self.creds.client_email,
            scope: "https://www.googleapis.com/auth/androidpublisher",
            aud: &self.creds.token_uri,
            iat: now,
            exp: now + 3600,
        };

        let key = EncodingKey::from_rsa_pem(self.creds.private_key.as_bytes())
            .map_err(|e| PlayError::Auth(format!("bad private key: {e}")))?;
        let assertion = encode(&Header::new(Algorithm::RS256), &claims, &key)
            .map_err(|e| PlayError::Auth(format!("jwt encode: {e}")))?;

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        // Build the urlencoded grant body (reqwest is built without the `.form`
        // helper's default features here, so encode it ourselves).
        let form_body = serde_urlencoded::to_string([
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ])
        .map_err(|e| PlayError::Auth(e.to_string()))?;

        let resp = self
            .http
            .post(&self.creds.token_uri)
            .header(
                axum::http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form_body)
            .send()
            .await
            .map_err(|e| PlayError::Auth(e.to_string()))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(PlayError::Auth(format!("token endpoint: {body}")));
        }

        let token: TokenResponse = resp
            .json()
            .await
            .map_err(|e| PlayError::Auth(e.to_string()))?;
        Ok(token.access_token)
    }
}

impl PlayBilling for GooglePlayBilling {
    async fn defer(&self, req: &DeferRequest) -> Result<DeferOutcome, PlayError> {
        let token = self.access_token().await?;

        let desired = req.current_expiry_ms + super::REWARD_DAYS * 24 * 60 * 60 * 1000;

        // LEGACY androidpublisher v3 defer endpoint (see module warning). Path
        // segments (purchase tokens especially) can contain reserved characters,
        // so percent-encode each one.
        let url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{pkg}/purchases/subscriptions/{sub}/tokens/{token}:defer",
            pkg = encode_segment(&req.package_name),
            sub = encode_segment(&req.subscription_id),
            token = encode_segment(&req.purchase_token),
        );

        let body = serde_json::json!({
            "deferralInfo": {
                "expectedExpiryTimeMillis": req.current_expiry_ms.to_string(),
                "desiredExpiryTimeMillis": desired.to_string(),
            }
        });

        #[derive(Deserialize)]
        struct DeferResponse {
            #[serde(rename = "newExpiryTimeMillis")]
            new_expiry_time_millis: Option<String>,
        }

        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| PlayError::Api(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PlayError::Api(format!("{status}: {text}")));
        }

        let parsed: DeferResponse = resp
            .json()
            .await
            .map_err(|e| PlayError::Api(e.to_string()))?;
        let new_expiry_ms = parsed
            .new_expiry_time_millis
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(desired);

        Ok(DeferOutcome { new_expiry_ms })
    }
}

/// Percent-encode a single URL path segment (encode everything outside the
/// unreserved set `A-Z a-z 0-9 - . _ ~`), so reserved characters in a purchase
/// token or product id can't break out of the segment.
fn encode_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::encode_segment;

    #[test]
    fn encodes_reserved_path_characters() {
        assert_eq!(encode_segment("abcABC123-._~"), "abcABC123-._~");
        assert_eq!(encode_segment("a/b?c#d"), "a%2Fb%3Fc%23d");
        assert_eq!(encode_segment("tok+en=x"), "tok%2Ben%3Dx");
    }
}
