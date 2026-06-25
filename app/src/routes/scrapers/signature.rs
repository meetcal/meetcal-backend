//! Slack request signature verification.
//!
//! Slack signs every request with `v0=HMAC_SHA256(signing_secret, "v0:" + timestamp + ":" + body)`.
//! See <https://api.slack.com/authentication/verifying-requests-from-slack>.

use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Extract the Slack signature headers and verify them over `body`. Returns
/// `Some(response)` with a ready-to-send `401` on failure, `None` on success.
/// Shared by the slash-command and interaction handlers.
pub fn require_valid(secret: &str, headers: &HeaderMap, body: &[u8]) -> Option<Response> {
    let timestamp = header(headers, "x-slack-request-timestamp");
    let provided = header(headers, "x-slack-signature");
    let (Some(timestamp), Some(provided)) = (timestamp, provided) else {
        return Some((StatusCode::UNAUTHORIZED, "missing Slack signature headers").into_response());
    };
    if verify(secret, &timestamp, body, &provided) {
        None
    } else {
        Some((StatusCode::UNAUTHORIZED, "bad Slack signature").into_response())
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_string)
}

/// Reject requests whose timestamp is more than this many seconds from now,
/// to blunt replay attacks (Slack's own recommended window).
const MAX_SKEW_SECS: i64 = 60 * 5;

/// Verify a Slack request signature over the raw request body.
///
/// Returns `true` only when the timestamp is fresh AND the HMAC matches. The
/// MAC comparison is constant-time via `verify_slice`.
pub fn verify(secret: &str, timestamp: &str, body: &[u8], signature: &str) -> bool {
    if secret.is_empty() {
        return false;
    }
    if !timestamp_is_fresh(timestamp) {
        return false;
    }

    let Some(hex_sig) = signature.strip_prefix("v0=") else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };

    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(b"v0:");
    mac.update(timestamp.as_bytes());
    mac.update(b":");
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

fn timestamp_is_fresh(timestamp: &str) -> bool {
    let Ok(ts) = timestamp.parse::<i64>() else {
        return false;
    };
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return false;
    };
    (now.as_secs() as i64 - ts).abs() <= MAX_SKEW_SECS
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    fn sign(secret: &str, ts: &str, body: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("v0:{ts}:{body}").as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn now() -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string()
    }

    #[test]
    fn accepts_valid_signature() {
        let secret = "shhh";
        let ts = now();
        let body = "command=%2Flist&text=";
        let sig = sign(secret, &ts, body);
        assert!(verify(secret, &ts, body.as_bytes(), &sig));
    }

    #[test]
    fn rejects_tampered_body() {
        let secret = "shhh";
        let ts = now();
        let sig = sign(secret, &ts, "command=%2Flist");
        assert!(!verify(secret, &ts, b"command=%2Fdelete", &sig));
    }

    #[test]
    fn rejects_stale_timestamp() {
        let secret = "shhh";
        let body = "x=1";
        let stale = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - (MAX_SKEW_SECS + 60))
            .to_string();
        let sig = sign(secret, &stale, body);
        assert!(!verify(secret, &stale, body.as_bytes(), &sig));
    }

    #[test]
    fn rejects_empty_secret() {
        assert!(!verify("", &now(), b"x=1", "v0=deadbeef"));
    }
}
