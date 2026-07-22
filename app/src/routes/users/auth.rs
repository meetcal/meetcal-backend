//! Clerk JWT authentication.
//!
//! Every authenticated request carries a `Bearer` Clerk session JWT. We extract
//! the `sub` (Clerk user id) and feed it to Postgres RLS via
//! `request.jwt.claim.sub`.
//!
//! ## Verification
//!
//! When `AuthConfig::enabled` is true we verify the token's RS256 signature
//! against Clerk's JWKS (fetched and cached, refreshed on an unknown `kid`) and
//! validate `exp` + `iss`. This is the production path and MUST be on in prod —
//! without it a forged token can impersonate any user and farm referral rewards.
//!
//! When disabled (dev/test only — see `configuration.yaml`), we fall back to the
//! legacy behaviour of trusting the base64 payload's `sub` without checking the
//! signature, so the unsigned tokens the test suite uses keep working.

use crate::AppError;
use crate::configuration::AuthSettings;
use axum::http::HeaderMap;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: String,
}

/// Runtime auth configuration + JWKS cache, held in `AppState`.
#[derive(Clone)]
pub struct AuthConfig {
    enabled: bool,
    issuer: String,
    jwks_url: String,
    keys: Arc<RwLock<KeyCache>>,
    http: reqwest::Client,
}

#[derive(Default)]
struct KeyCache {
    /// kid -> decoding key.
    keys: HashMap<String, DecodingKey>,
    /// When we last fetched JWKS, to rate-limit refresh on unknown-kid misses.
    last_refresh: Option<Instant>,
}

/// Don't refetch JWKS more than once per this window when a `kid` misses, so a
/// flood of bogus tokens can't turn into a fetch storm.
const JWKS_MIN_REFRESH: Duration = Duration::from_secs(60);

/// Proactively re-fetch JWKS once the cache is older than this, so rotated keys
/// are picked up even without an unknown-kid miss.
const JWKS_TTL: Duration = Duration::from_secs(15 * 60);

/// Explicit dev/test escape hatch that permits running WITHOUT JWT verification.
/// Absent this, a config with verification disabled PANICS at startup — the
/// service fails closed rather than silently accepting forged tokens.
const ALLOW_UNVERIFIED_ENV: &str = "APP_ALLOW_UNVERIFIED_JWT";

/// Pure startup decision, split out for testing. `Ok(true)` => verify,
/// `Ok(false)` => explicitly unverified (dev/test), `Err(_)` => refuse to start.
fn decide_verification(
    verification_enabled: bool,
    allow_unverified: bool,
    jwks_url: &str,
    issuer: &str,
) -> Result<bool, &'static str> {
    if allow_unverified {
        // Explicit opt-out (test harness / local dev only).
        return Ok(false);
    }
    if !verification_enabled {
        return Err(
            "JWT verification is disabled but APP_ALLOW_UNVERIFIED_JWT is not set — refusing to \
             start (fail closed). Enable auth.jwt_verification_enabled in production.",
        );
    }
    if jwks_url.is_empty() || issuer.is_empty() {
        return Err(
            "auth.jwt_verification_enabled is true but auth.jwks_url / auth.issuer are not set",
        );
    }
    Ok(true)
}

impl AuthConfig {
    pub fn from_settings(settings: &AuthSettings) -> Self {
        let allow_unverified = std::env::var(ALLOW_UNVERIFIED_ENV).as_deref() == Ok("1");
        let enabled = match decide_verification(
            settings.jwt_verification_enabled,
            allow_unverified,
            &settings.jwks_url,
            &settings.issuer,
        ) {
            Ok(enabled) => enabled,
            Err(reason) => panic!("{reason}"),
        };

        Self {
            enabled,
            issuer: settings.issuer.clone(),
            jwks_url: settings.jwks_url.clone(),
            keys: Arc::new(RwLock::new(KeyCache::default())),
            http: reqwest::Client::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Resolve and validate the caller's user id from the request headers.
    pub async fn user_id(&self, headers: &HeaderMap) -> Result<String, AppError> {
        let token = bearer_token(headers)?;

        if !self.enabled {
            return unverified_sub(token);
        }

        self.verified_sub(token).await
    }

    async fn verified_sub(&self, token: &str) -> Result<String, AppError> {
        let header = decode_header(token).map_err(|_| AppError::Unauthorized)?;
        let kid = header.kid.ok_or(AppError::Unauthorized)?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[self.issuer.as_str()]);
        // Clerk session tokens don't carry a stable `aud`; we validate iss + exp.
        validation.validate_aud = false;
        validation.validate_exp = true;

        // Proactively refresh a stale cache (TTL), so rotated keys are picked up
        // even without an unknown-kid miss. Ignore refresh errors here and fall
        // back to whatever is cached.
        if self.cache_is_stale() {
            let _ = self.refresh_keys().await;
        }

        // Try the cached key first, then refresh once on a miss (key rotation).
        for attempt in 0..2 {
            let key = self.key_for(&kid);
            let key = match key {
                Some(key) => key,
                None => {
                    if attempt == 0 {
                        self.refresh_keys().await?;
                        continue;
                    }
                    return Err(AppError::Unauthorized);
                }
            };

            return match decode::<JwtClaims>(token, &key, &validation) {
                Ok(data) if !data.claims.sub.trim().is_empty() => Ok(data.claims.sub),
                _ => Err(AppError::Unauthorized),
            };
        }

        Err(AppError::Unauthorized)
    }

    fn key_for(&self, kid: &str) -> Option<DecodingKey> {
        self.keys.read().ok()?.keys.get(kid).cloned()
    }

    /// True when the JWKS cache has never been fetched or is older than the TTL.
    fn cache_is_stale(&self) -> bool {
        match self.keys.read() {
            Ok(cache) => match cache.last_refresh {
                Some(last) => last.elapsed() >= JWKS_TTL,
                None => true,
            },
            Err(_) => true,
        }
    }

    async fn refresh_keys(&self) -> Result<(), AppError> {
        // Rate-limit refreshes so bogus-kid floods don't hammer Clerk.
        if let Ok(cache) = self.keys.read()
            && let Some(last) = cache.last_refresh
            && last.elapsed() < JWKS_MIN_REFRESH
        {
            return Ok(());
        }

        let jwks: jsonwebtoken::jwk::JwkSet = self
            .http
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|_| AppError::Unauthorized)?
            .json()
            .await
            .map_err(|_| AppError::Unauthorized)?;

        let mut fresh = HashMap::new();
        for jwk in &jwks.keys {
            if let Some(kid) = jwk.common.key_id.clone()
                && let Ok(key) = DecodingKey::from_jwk(jwk)
            {
                fresh.insert(kid, key);
            }
        }

        if let Ok(mut cache) = self.keys.write() {
            cache.keys = fresh;
            cache.last_refresh = Some(Instant::now());
        }

        Ok(())
    }
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, AppError> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())
        .ok_or(AppError::Unauthorized)
}

/// UNVERIFIED extraction of `sub` from the JWT payload — dev/test only. Trusts
/// the token without checking the signature.
fn unverified_sub(token: &str) -> Result<String, AppError> {
    let payload = token.split('.').nth(1).ok_or(AppError::Unauthorized)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| AppError::Unauthorized)?;
    let claims: JwtClaims = serde_json::from_slice(&decoded).map_err(|_| AppError::Unauthorized)?;

    if claims.sub.trim().is_empty() {
        return Err(AppError::Unauthorized);
    }

    Ok(claims.sub)
}

/// Legacy free-function extractor kept for callers that only need the unverified
/// `sub` (none currently do the verified path without `AuthConfig`). Prefer
/// `AuthConfig::user_id`.
pub fn user_id_from_headers(headers: &HeaderMap) -> Result<String, AppError> {
    let token = bearer_token(headers)?;
    unverified_sub(token)
}

pub async fn set_request_user(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
) -> Result<(), AppError> {
    sqlx::query("SELECT set_config('request.jwt.claim.sub', $1, true)")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Mark the current transaction as a server-authoritative (backend) operation so
/// RLS policies gated on `app_is_backend()` apply. Used by the webhook, the
/// qualification cron, and the redeem cross-user code lookup. Never set from a
/// path that should be scoped to a single end user's own rows.
pub async fn set_backend_context(tx: &mut Transaction<'_, Postgres>) -> Result<(), AppError> {
    sqlx::query("SELECT set_config('request.meetcal.backend', 'on', true)")
        .execute(&mut **tx)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::decide_verification;

    #[test]
    fn verify_when_enabled_with_config() {
        assert_eq!(
            decide_verification(true, false, "https://jwks", "https://iss"),
            Ok(true)
        );
    }

    #[test]
    fn fails_closed_when_disabled_without_hatch() {
        // The critical case: disabled config, no escape hatch -> refuse to start.
        assert!(decide_verification(false, false, "", "").is_err());
    }

    #[test]
    fn escape_hatch_allows_unverified() {
        // Test harness path: explicit opt-out runs unverified.
        assert_eq!(decide_verification(false, true, "", ""), Ok(false));
        // Hatch wins even if config says enabled (keeps shared config usable).
        assert_eq!(
            decide_verification(true, true, "https://jwks", "https://iss"),
            Ok(false)
        );
    }

    #[test]
    fn enabled_but_missing_jwks_or_issuer_refuses() {
        assert!(decide_verification(true, false, "", "https://iss").is_err());
        assert!(decide_verification(true, false, "https://jwks", "").is_err());
    }
}
