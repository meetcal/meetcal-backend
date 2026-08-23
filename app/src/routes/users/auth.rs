use crate::AppError;
use axum::http::HeaderMap;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use sqlx::{Postgres, Transaction};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, RwLock};

const MIN_JWKS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: String,
    #[allow(dead_code)]
    exp: u64,
    iss: String,
    azp: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    n: String,
    e: String,
    alg: Option<String>,
    #[serde(rename = "use")]
    key_use: Option<String>,
}

/// Verifies Clerk session tokens against Clerk's rotating RS256 public keys.
pub struct AuthVerifier {
    jwks_url: Option<String>,
    issuer: String,
    authorized_parties: Vec<String>,
    audience: Option<String>,
    client: reqwest::Client,
    keys: RwLock<HashMap<String, Arc<DecodingKey>>>,
    last_refresh: Mutex<Option<std::time::Instant>>,
}

impl AuthVerifier {
    pub fn from_env() -> anyhow::Result<Option<Arc<Self>>> {
        let jwks_url = std::env::var("CLERK_JWKS_URL").ok();
        let issuer = std::env::var("CLERK_ISSUER").ok();
        let authorized_parties = std::env::var("CLERK_AUTHORIZED_PARTIES").ok();

        if jwks_url.is_none() && issuer.is_none() && authorized_parties.is_none() {
            return Ok(None);
        }

        let jwks_url = jwks_url.ok_or_else(|| anyhow::anyhow!("CLERK_JWKS_URL is required"))?;
        let issuer = issuer.ok_or_else(|| anyhow::anyhow!("CLERK_ISSUER is required"))?;
        let authorized_parties = authorized_parties
            .ok_or_else(|| anyhow::anyhow!("CLERK_AUTHORIZED_PARTIES is required"))?
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();

        if authorized_parties.is_empty() {
            anyhow::bail!("CLERK_AUTHORIZED_PARTIES must contain at least one origin");
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;

        Ok(Some(Arc::new(Self {
            jwks_url: Some(jwks_url),
            issuer,
            authorized_parties,
            audience: std::env::var("CLERK_AUDIENCE").ok(),
            client,
            keys: RwLock::new(HashMap::new()),
            last_refresh: Mutex::new(None),
        })))
    }

    /// Creates a verifier with a fixed test/development key and no network access.
    pub fn from_rsa_pem(
        kid: &str,
        public_key_pem: &[u8],
        issuer: &str,
        authorized_parties: Vec<String>,
        audience: Option<String>,
    ) -> anyhow::Result<Arc<Self>> {
        let key = DecodingKey::from_rsa_pem(public_key_pem)?;
        Ok(Arc::new(Self {
            jwks_url: None,
            issuer: issuer.to_owned(),
            authorized_parties,
            audience,
            client: reqwest::Client::new(),
            keys: RwLock::new(HashMap::from([(kid.to_owned(), Arc::new(key))])),
            last_refresh: Mutex::new(None),
        }))
    }

    async fn refresh_keys(&self) -> Result<(), ()> {
        let url = self.jwks_url.as_ref().ok_or(())?;
        let mut last_refresh = self.last_refresh.lock().await;
        if last_refresh.is_some_and(|instant| instant.elapsed() < MIN_JWKS_REFRESH_INTERVAL) {
            return Ok(());
        }
        *last_refresh = Some(std::time::Instant::now());
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| ())?
            .error_for_status()
            .map_err(|_| ())?
            .json::<JwksResponse>()
            .await
            .map_err(|_| ())?;

        let mut next_keys = HashMap::new();
        for jwk in response.keys {
            if jwk.kty != "RSA"
                || jwk.alg.as_deref().is_some_and(|alg| alg != "RS256")
                || jwk.key_use.as_deref().is_some_and(|usage| usage != "sig")
            {
                continue;
            }
            if let Ok(key) = DecodingKey::from_rsa_components(&jwk.n, &jwk.e) {
                next_keys.insert(jwk.kid, Arc::new(key));
            }
        }

        if next_keys.is_empty() {
            return Err(());
        }
        *self.keys.write().await = next_keys;
        Ok(())
    }

    async fn verify(&self, token: &str) -> Result<String, AppError> {
        let header = decode_header(token).map_err(|_| AppError::Unauthorized)?;
        if header.alg != Algorithm::RS256 {
            return Err(AppError::Unauthorized);
        }
        let kid = header.kid.ok_or(AppError::Unauthorized)?;

        let mut key = self.keys.read().await.get(&kid).cloned();
        if key.is_none() {
            self.refresh_keys()
                .await
                .map_err(|_| AppError::Unauthorized)?;
            key = self.keys.read().await.get(&kid).cloned();
        }
        let key = key.ok_or(AppError::Unauthorized)?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_required_spec_claims(&["exp", "iss", "sub"]);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.validate_nbf = true;
        if let Some(audience) = self.audience.as_deref() {
            validation.set_audience(&[audience]);
        } else {
            validation.validate_aud = false;
        }

        let claims = decode::<JwtClaims>(token, &key, &validation)
            .map_err(|_| AppError::Unauthorized)?
            .claims;

        if claims.sub.trim().is_empty() || claims.iss != self.issuer {
            return Err(AppError::Unauthorized);
        }
        if let Some(azp) = claims.azp
            && !self
                .authorized_parties
                .iter()
                .any(|allowed| allowed == &azp)
        {
            return Err(AppError::Unauthorized);
        }

        Ok(claims.sub)
    }
}

pub async fn user_id_from_headers(
    headers: &HeaderMap,
    verifier: Option<&AuthVerifier>,
) -> Result<String, AppError> {
    let verifier = verifier.ok_or(AppError::Unauthorized)?;
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;

    verifier.verify(token).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
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

    const KID: &str = "meetcal-test-key";
    const ISSUER: &str = "https://clerk.test";
    const AZP: &str = "https://meetcal.app";

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
    struct Claims<'a> {
        sub: &'a str,
        iss: &'a str,
        azp: &'a str,
        exp: u64,
        nbf: u64,
    }

    fn token(issuer: &str, azp: &str, expired: bool) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(KID.to_string());
        encode(
            &header,
            &Claims {
                sub: "user_123",
                iss: issuer,
                azp,
                exp: if expired { now - 300 } else { now + 300 },
                nbf: now - 1,
            },
            &EncodingKey::from_rsa_pem(TEST_KEYS.private_pem.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn verifier() -> Arc<AuthVerifier> {
        AuthVerifier::from_rsa_pem(
            KID,
            TEST_KEYS.public_pem.as_bytes(),
            ISSUER,
            vec![AZP.to_string()],
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn accepts_a_valid_signed_token() {
        assert_eq!(
            verifier().verify(&token(ISSUER, AZP, false)).await.unwrap(),
            "user_123"
        );
    }

    #[tokio::test]
    async fn rejects_forged_expired_and_wrong_origin_tokens() {
        let verifier = verifier();
        assert!(
            verifier
                .verify("e30.eyJzdWIiOiJhdHRhY2tlciJ9.forged")
                .await
                .is_err()
        );
        assert!(verifier.verify(&token(ISSUER, AZP, true)).await.is_err());
        assert!(
            verifier
                .verify(&token(ISSUER, "https://evil.example", false))
                .await
                .is_err()
        );
        assert!(
            verifier
                .verify(&token("https://wrong-issuer.test", AZP, false))
                .await
                .is_err()
        );
    }
}
