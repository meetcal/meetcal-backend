//! Apple StoreKit promotional-offer signature generation.
//!
//! For an existing subscriber, a free reward month is delivered as an Apple
//! *promotional offer* that applies at the next renewal. The app presents the
//! offer using a signature this backend generates with the subscription-signing
//! key (a P-256 / ECDSA key downloaded from App Store Connect as a `.p8`).
//!
//! Signature spec (StoreKit `SKPaymentDiscount`): sign, with ECDSA-SHA256, the
//! UTF-8 of the fields joined by U+2063 (INVISIBLE SEPARATOR):
//!
//!   appBundleID ⁣ keyID ⁣ productID ⁣ offerID ⁣ applicationUsername ⁣ nonce ⁣ timestamp
//!
//! then base64-encode the DER signature. `nonce` is a lowercase UUID; `timestamp`
//! is milliseconds since epoch. The app MUST set `applicationUsername` to the
//! same (lowercased) user id used here.
//!
//! See <https://developer.apple.com/documentation/storekit/generating-a-signature-for-promotional-offers>.

use crate::routes::referrals::parse_kv_map;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::pkcs8::DecodePrivateKey;
use serde::Serialize;
use std::collections::HashMap;

/// U+2063 INVISIBLE SEPARATOR — Apple's required field delimiter.
const SEP: char = '\u{2063}';

#[derive(Clone)]
pub struct AppleConfig {
    pub key_id: String,
    pub bundle_id: String,
    /// PEM contents of the `.p8` signing key.
    private_key_pem: String,
    /// product_id -> offer_id.
    offers: HashMap<String, String>,
}

#[derive(Debug)]
pub enum AppleError {
    /// No offer id configured for the product the user is subscribed to.
    NoOfferForProduct,
    /// The configured `.p8` key could not be parsed / used.
    KeyError,
}

/// The signed bundle the iOS client needs to present the promotional offer.
#[derive(Debug, Serialize)]
pub struct OfferBundle {
    pub product_id: String,
    pub offer_id: String,
    pub key_id: String,
    pub nonce: String,
    pub timestamp: i64,
    pub signature: String,
}

impl AppleConfig {
    pub fn from_env() -> Option<Self> {
        let key_id = std::env::var("APPLE_IAP_KEY_ID").unwrap_or_default();
        let bundle_id = std::env::var("APPLE_BUNDLE_ID").unwrap_or_default();
        // Env vars often carry `\n` as a literal escape; normalize to real newlines.
        let private_key_pem = std::env::var("APPLE_IAP_PRIVATE_KEY")
            .unwrap_or_default()
            .replace("\\n", "\n");
        let offers = parse_kv_map(&std::env::var("APPLE_IAP_OFFER_IDS").unwrap_or_default());

        if key_id.is_empty() || bundle_id.is_empty() || private_key_pem.trim().is_empty() {
            return None;
        }

        Some(Self {
            key_id,
            bundle_id,
            private_key_pem,
            offers,
        })
    }

    /// Generate a promotional-offer signature for `product_id` and `app_username`.
    pub fn sign_offer(
        &self,
        product_id: &str,
        app_username: &str,
    ) -> Result<OfferBundle, AppleError> {
        let offer_id = self
            .offers
            .get(product_id)
            .ok_or(AppleError::NoOfferForProduct)?
            .clone();

        let nonce = uuid::Uuid::new_v4().to_string(); // lowercase, hyphenated
        let timestamp = chrono::Utc::now().timestamp_millis();
        // Apple recommends a lowercased application username; the app must match.
        let username = app_username.to_ascii_lowercase();

        let payload = format!(
            "{bundle}{SEP}{key}{SEP}{product}{SEP}{offer}{SEP}{user}{SEP}{nonce}{SEP}{ts}",
            bundle = self.bundle_id,
            key = self.key_id,
            product = product_id,
            offer = offer_id,
            user = username,
            nonce = nonce,
            ts = timestamp,
        );

        let signing_key =
            SigningKey::from_pkcs8_pem(&self.private_key_pem).map_err(|_| AppleError::KeyError)?;
        // ECDSA-SHA256 (RFC 6979 deterministic) over the payload bytes.
        let signature: Signature = signing_key.sign(payload.as_bytes());
        let signature = STANDARD.encode(signature.to_der().as_bytes());

        Ok(OfferBundle {
            product_id: product_id.to_string(),
            offer_id,
            key_id: self.key_id.clone(),
            nonce,
            timestamp,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{VerifyingKey, signature::Verifier};
    use p256::pkcs8::EncodePublicKey;

    // A self-generated P-256 key in PKCS#8 PEM, mirroring an Apple `.p8`.
    fn test_config() -> (AppleConfig, VerifyingKey) {
        use p256::pkcs8::EncodePrivateKey;
        // Deterministic-ish key for the test.
        let signing = SigningKey::from_bytes(&[7u8; 32].into()).unwrap();
        let pem = signing
            .to_pkcs8_pem(Default::default())
            .unwrap()
            .to_string();
        let verifying = *signing.verifying_key();

        let mut offers = HashMap::new();
        offers.insert("com.meetcal.monthly".to_string(), "free_month".to_string());

        (
            AppleConfig {
                key_id: "ABC123".to_string(),
                bundle_id: "app.meetcal".to_string(),
                private_key_pem: pem,
                offers,
            },
            verifying,
        )
    }

    #[test]
    fn signs_and_verifies() {
        let (cfg, verifying) = test_config();
        let bundle = cfg.sign_offer("com.meetcal.monthly", "USER_2xYz").unwrap();

        assert_eq!(bundle.offer_id, "free_month");
        assert_eq!(bundle.key_id, "ABC123");
        assert!(!bundle.signature.is_empty());

        // Rebuild the exact payload and verify the DER signature.
        let payload = format!(
            "app.meetcal{SEP}ABC123{SEP}com.meetcal.monthly{SEP}free_month{SEP}user_2xyz{SEP}{}{SEP}{}",
            bundle.nonce, bundle.timestamp,
        );
        let der = STANDARD.decode(&bundle.signature).unwrap();
        let sig = Signature::from_der(&der).unwrap();
        assert!(verifying.verify(payload.as_bytes(), &sig).is_ok());

        // Sanity: the public key encodes (exercises the key path).
        assert!(verifying.to_public_key_der().is_ok());
    }

    #[test]
    fn unknown_product_is_rejected() {
        let (cfg, _) = test_config();
        assert!(matches!(
            cfg.sign_offer("com.meetcal.unknown", "user"),
            Err(AppleError::NoOfferForProduct)
        ));
    }
}
