#![allow(dead_code)]

use app::{
    common::spawn_server::{TestApp, spawn_app_as_api_role, spawn_app_with_auth},
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
    /// Web sessions carry the origin here; native Clerk sessions omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    azp: Option<&'a str>,
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

/// The test app with every query running as `meetcal_api`, the role
/// production connects as: its grants and row-level security apply.
pub async fn spawn_test_app_as_api_role() -> TestApp {
    let auth = AuthVerifier::from_rsa_pem(
        TEST_KID,
        TEST_KEYS.public_pem.as_bytes(),
        TEST_ISSUER,
        vec![TEST_AZP.to_string()],
        None,
    )
    .expect("valid test public key");
    spawn_app_as_api_role(Some(auth)).await
}

pub fn test_token(user_id: &str) -> String {
    test_token_with(user_id, TEST_ISSUER, TEST_AZP, 300)
}

pub fn test_token_with(user_id: &str, issuer: &str, azp: &str, lifetime_secs: i64) -> String {
    test_token_claims(user_id, issuer, Some(azp), lifetime_secs)
}

/// A token shaped like a native app session: valid signature, no `azp`.
pub fn test_token_without_azp(user_id: &str, issuer: &str) -> String {
    test_token_claims(user_id, issuer, None, 300)
}

fn test_token_claims(user_id: &str, issuer: &str, azp: Option<&str>, lifetime_secs: i64) -> String {
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

/// The test database URL, resolved the same way the spawned server does.
pub fn database_url() -> String {
    app::load_env();
    match std::env::var("DATABASE_URL") {
        Ok(database_url) => database_url,
        Err(_) => app::configuration::get_configuration()
            .expect("Failed to read config")
            .database
            .connection_string()
            .expect("Failed to build database connection string"),
    }
}

/// A direct pool on the test database, for seeding rows and for checking what
/// the `meetcal_api` role can do (`SET ROLE`) without going through the API.
/// Resolves the URL the same way the spawned server does.
pub async fn db_pool() -> sqlx::PgPool {
    let database_url = database_url();
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("Failed to connect to postgres")
}
