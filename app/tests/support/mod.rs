#![allow(dead_code)]

use app::{
    common::spawn_server::{TestApp, spawn_app_with_auth},
    routes::users::auth::AuthVerifier,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rand::rngs::OsRng;
use rsa::{
    RsaPrivateKey, RsaPublicKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
};
use serde::Serialize;
use std::{
    sync::LazyLock,
    time::{SystemTime, UNIX_EPOCH},
};

const TEST_ISSUER: &str = "https://clerk.test";
const TEST_AZP: &str = "https://meetcal.app";
const TEST_KID: &str = "meetcal-test-key";

struct TestKeys {
    private_pem: String,
    public_pem: String,
}

static TEST_KEYS: LazyLock<TestKeys> = LazyLock::new(|| {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).expect("generate test RSA key");
    let public = RsaPublicKey::from(&private);
    TestKeys {
        private_pem: private
            .to_pkcs8_pem(LineEnding::LF)
            .expect("encode test private key")
            .to_string(),
        public_pem: public
            .to_public_key_pem(LineEnding::LF)
            .expect("encode test public key"),
    }
});

#[derive(Serialize)]
struct TestClaims<'a> {
    sub: &'a str,
    iss: &'a str,
    azp: &'a str,
    exp: u64,
    nbf: u64,
}

pub async fn spawn_test_app() -> TestApp {
    let auth = AuthVerifier::from_rsa_pem(
        TEST_KID,
        TEST_KEYS.public_pem.as_bytes(),
        TEST_ISSUER,
        vec![TEST_AZP.to_string()],
        None,
    )
    .expect("valid test public key");
    spawn_app_with_auth(Some(auth)).await
}

pub fn test_token(user_id: &str) -> String {
    test_token_with(user_id, TEST_ISSUER, TEST_AZP, 300)
}

pub fn test_token_with(user_id: &str, issuer: &str, azp: &str, lifetime_secs: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs();
    let exp = if lifetime_secs >= 0 {
        now + lifetime_secs as u64
    } else {
        now.saturating_sub(lifetime_secs.unsigned_abs())
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(TEST_KID.to_string());
    encode(
        &header,
        &TestClaims {
            sub: user_id,
            iss: issuer,
            azp,
            exp,
            nbf: now.saturating_sub(1),
        },
        &EncodingKey::from_rsa_pem(TEST_KEYS.private_pem.as_bytes())
            .expect("valid test private key"),
    )
    .expect("encode test token")
}
