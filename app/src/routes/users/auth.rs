use crate::AppError;
use axum::http::HeaderMap;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use sqlx::{Postgres, Transaction};
use std::{
    collections::HashMap,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock};

/// Floor between two JWKS fetches after a *successful* one. An unknown `kid`
/// inside this window is answered from the cache (i.e. rejected) rather than
/// re-fetched, so a flood of tokens with bogus key ids cannot hammer Clerk.
const MIN_JWKS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// Negative cache after a *failed* fetch. Short on purpose: one transient
/// error must not lock a freshly rotated key out for the full refresh
/// interval, but the retry rate is still bounded while Clerk is down.
const JWKS_FAILURE_BACKOFF: Duration = Duration::from_secs(5);
/// Ceiling on one JWKS fetch. Clerk is a third party on the auth path, so a
/// hung fetch must not hold the request open to the 15s server timeout.
const JWKS_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// Ceiling on how many keys one JWKS response may contribute. Clerk publishes
/// the current signing key plus any mid-rotation predecessors -- a handful.
/// The response is a third party's array, so the loop that walks it declares a
/// bound rather than trusting the body's length.
const MAX_JWKS_KEYS: usize = 16;

/// Only the claims this code inspects. `exp` / `nbf` are enforced by
/// `jsonwebtoken`'s own validation (see [`AuthVerifier::verify`]), which
/// deserializes them separately, so they are deliberately absent here.
#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: String,
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
    refresh: Mutex<RefreshClock>,
}

/// When the JWKS was last fetched, kept separately for success and failure so
/// each can drive its own throttle (see [`MIN_JWKS_REFRESH_INTERVAL`] and
/// [`JWKS_FAILURE_BACKOFF`]).
#[derive(Debug, Default)]
struct RefreshClock {
    last_success: Option<Instant>,
    last_failure: Option<Instant>,
}

/// Why a JWKS refresh produced no new keys. Logged with the reason; the caller
/// still answers the request with a plain `401`, since the token cannot be
/// verified either way.
#[derive(Debug)]
enum JwksRefreshError {
    /// The verifier was built with a fixed key set and has no JWKS URL.
    NotConfigured,
    /// A fetch failed less than [`JWKS_FAILURE_BACKOFF`] ago; not retried.
    BackingOff,
    /// The HTTP request itself failed (DNS, connect, timeout).
    Request(reqwest::Error),
    /// Clerk answered with a non-2xx status.
    Status(reqwest::StatusCode),
    /// The 2xx body was not a JWKS document.
    Decode(reqwest::Error),
    /// The document parsed but held no RS256 signing key.
    NoUsableKeys,
}

impl fmt::Display for JwksRefreshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "no JWKS URL configured"),
            Self::BackingOff => write!(
                f,
                "last fetch failed less than {}s ago",
                JWKS_FAILURE_BACKOFF.as_secs()
            ),
            Self::Request(error) => write!(f, "request failed: {error}"),
            Self::Status(status) => write!(f, "unexpected HTTP status {status}"),
            Self::Decode(error) => write!(f, "response was not a JWKS document: {error}"),
            Self::NoUsableKeys => write!(f, "response held no RS256 signing key"),
        }
    }
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
            .timeout(JWKS_FETCH_TIMEOUT)
            .build()?;

        Ok(Some(Arc::new(Self {
            jwks_url: Some(jwks_url),
            issuer,
            authorized_parties,
            audience: std::env::var("CLERK_AUDIENCE").ok(),
            client,
            keys: RwLock::new(HashMap::new()),
            refresh: Mutex::new(RefreshClock::default()),
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
            refresh: Mutex::new(RefreshClock::default()),
        }))
    }

    /// Re-fetch Clerk's key set, throttled by [`RefreshClock`]. The lock is held
    /// across the fetch so concurrent unknown-`kid` requests queue behind one
    /// network round trip instead of each starting their own.
    async fn refresh_keys(&self) -> Result<(), JwksRefreshError> {
        let url = self
            .jwks_url
            .as_ref()
            .ok_or(JwksRefreshError::NotConfigured)?;
        let mut clock = self.refresh.lock().await;
        if clock
            .last_success
            .is_some_and(|at| at.elapsed() < MIN_JWKS_REFRESH_INTERVAL)
        {
            return Ok(());
        }
        if clock
            .last_failure
            .is_some_and(|at| at.elapsed() < JWKS_FAILURE_BACKOFF)
        {
            return Err(JwksRefreshError::BackingOff);
        }

        match self.fetch_keys(url).await {
            Ok(next_keys) => {
                *self.keys.write().await = next_keys;
                // Stamped only now: a failed fetch must not start the 60s
                // refresh interval, or a rotated key stays unknown until it ends.
                clock.last_success = Some(Instant::now());
                clock.last_failure = None;
                Ok(())
            }
            Err(error) => {
                clock.last_failure = Some(Instant::now());
                // TODO: move to `tracing` once the crate adopts it.
                eprintln!("JWKS refresh from {url} failed: {error}");
                Err(error)
            }
        }
    }

    async fn fetch_keys(
        &self,
        url: &str,
    ) -> Result<HashMap<String, Arc<DecodingKey>>, JwksRefreshError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(JwksRefreshError::Request)?;
        let status = response.status();
        if !status.is_success() {
            return Err(JwksRefreshError::Status(status));
        }
        let document = response
            .json::<JwksResponse>()
            .await
            .map_err(JwksRefreshError::Decode)?;

        let next_keys = usable_signing_keys(document.keys);
        if next_keys.is_empty() {
            return Err(JwksRefreshError::NoUsableKeys);
        }
        Ok(next_keys)
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
            // `jsonwebtoken` only compares `aud` when the claim is present. A
            // native Clerk session token carries no `azp` (that claim is the
            // web origin), so once an audience is configured `aud` is the only
            // claim binding the token to this API and it must be there.
            validation.set_required_spec_claims(&["exp", "iss", "sub", "aud"]);
        } else {
            validation.validate_aud = false;
        }

        let claims = decode::<JwtClaims>(token, &key, &validation)
            .map_err(|_| AppError::Unauthorized)?
            .claims;

        if claims.sub.trim().is_empty() || claims.iss != self.issuer {
            return Err(AppError::Unauthorized);
        }
        // `azp` is Clerk's web origin claim: present on browser sessions (and
        // then it must be a listed party), absent on native app sessions.
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

/// Keep the RS256 signing keys from a JWKS response, at most [`MAX_JWKS_KEYS`]
/// of them. Split out from `refresh_keys` so the bound is testable without a
/// network round trip.
fn usable_signing_keys(keys: Vec<Jwk>) -> HashMap<String, Arc<DecodingKey>> {
    let mut usable = HashMap::new();
    for jwk in keys.into_iter().take(MAX_JWKS_KEYS) {
        if jwk.kty != "RSA"
            || jwk.alg.as_deref().is_some_and(|alg| alg != "RS256")
            || jwk.key_use.as_deref().is_some_and(|usage| usage != "sig")
        {
            continue;
        }
        if let Ok(key) = DecodingKey::from_rsa_components(&jwk.n, &jwk.e) {
            usable.insert(jwk.kid, Arc::new(key));
        }
    }
    usable
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
        pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding},
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
        /// Absent on native Clerk session tokens, so the test helper can omit it.
        #[serde(skip_serializing_if = "Option::is_none")]
        azp: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        aud: Option<&'a str>,
        exp: u64,
        nbf: u64,
    }

    fn token(issuer: &str, azp: &str, expired: bool) -> String {
        token_with(KID, issuer, Some(azp), None, expired)
    }

    fn token_with(
        kid: &str,
        issuer: &str,
        azp: Option<&str>,
        aud: Option<&str>,
        expired: bool,
    ) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        encode(
            &header,
            &Claims {
                sub: "user_123",
                iss: issuer,
                azp,
                aud,
                exp: if expired { now - 300 } else { now + 300 },
                nbf: now - 1,
            },
            &EncodingKey::from_rsa_pem(TEST_KEYS.private_pem.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn verifier() -> Arc<AuthVerifier> {
        verifier_with_audience(None)
    }

    fn verifier_with_audience(audience: Option<&str>) -> Arc<AuthVerifier> {
        AuthVerifier::from_rsa_pem(
            KID,
            TEST_KEYS.public_pem.as_bytes(),
            ISSUER,
            vec![AZP.to_string()],
            audience.map(str::to_string),
        )
        .unwrap()
    }

    /// Modulus and exponent of the generated test key, so a synthetic JWKS
    /// entry actually decodes.
    fn rsa_components() -> (String, String) {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        use rsa::traits::PublicKeyParts;
        let public = RsaPublicKey::from(
            &RsaPrivateKey::from_pkcs8_pem(&TEST_KEYS.private_pem).expect("parse test key"),
        );
        (
            URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
            URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
        )
    }

    fn jwk(kid: &str, kty: &str, alg: Option<&str>) -> Jwk {
        let (n, e) = rsa_components();
        Jwk {
            kid: kid.to_string(),
            kty: kty.to_string(),
            n,
            e,
            alg: alg.map(str::to_string),
            key_use: Some("sig".to_string()),
        }
    }

    #[test]
    fn jwks_key_set_is_bounded_and_filtered() {
        // Zero: a JWKS with nothing usable leaves the cache untouched (the
        // caller treats an empty set as a refresh failure).
        assert!(usable_signing_keys(Vec::new()).is_empty());
        // One.
        assert_eq!(usable_signing_keys(vec![jwk("a", "RSA", None)]).len(), 1);
        // Many, mixed: non-RSA and non-RS256 entries are dropped.
        let mixed = vec![
            jwk("a", "RSA", Some("RS256")),
            jwk("b", "EC", None),
            jwk("c", "RSA", Some("HS256")),
        ];
        assert_eq!(usable_signing_keys(mixed).len(), 1);
        // Max: an oversized third-party response cannot grow the map without
        // bound.
        let oversized: Vec<Jwk> = (0..(MAX_JWKS_KEYS + 10))
            .map(|index| jwk(&format!("kid-{index}"), "RSA", Some("RS256")))
            .collect();
        assert_eq!(usable_signing_keys(oversized).len(), MAX_JWKS_KEYS);
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

    /// Native Clerk session tokens carry no `azp` (it is the web origin claim),
    /// so an absent `azp` must verify -- while every other check still applies.
    #[tokio::test]
    async fn absent_azp_is_accepted_but_other_claims_still_apply() {
        let verifier = verifier();
        assert_eq!(
            verifier
                .verify(&token_with(KID, ISSUER, None, None, false))
                .await
                .unwrap(),
            "user_123"
        );
        assert!(
            verifier
                .verify(&token_with(
                    KID,
                    "https://wrong-issuer.test",
                    None,
                    None,
                    false
                ))
                .await
                .is_err()
        );
        assert!(
            verifier
                .verify(&token_with(KID, ISSUER, None, None, true))
                .await
                .is_err()
        );
    }

    /// With `CLERK_AUDIENCE` set, `aud` is what ties an `azp`-less token to this
    /// API, so it must be present and match; `jsonwebtoken` alone would skip a
    /// missing `aud`.
    #[tokio::test]
    async fn configured_audience_is_required_and_must_match() {
        let with_audience = verifier_with_audience(Some("meetcal-api"));
        assert!(
            with_audience
                .verify(&token_with(KID, ISSUER, None, None, false))
                .await
                .is_err(),
            "missing aud must fail once an audience is configured"
        );
        assert!(
            with_audience
                .verify(&token_with(KID, ISSUER, Some(AZP), None, false))
                .await
                .is_err(),
            "a listed azp does not stand in for the audience"
        );
        assert!(
            with_audience
                .verify(&token_with(KID, ISSUER, None, Some("other-api"), false))
                .await
                .is_err()
        );
        assert_eq!(
            with_audience
                .verify(&token_with(KID, ISSUER, None, Some("meetcal-api"), false))
                .await
                .unwrap(),
            "user_123"
        );
        // Without a configured audience the claim is ignored, as before.
        assert!(
            verifier()
                .verify(&token_with(KID, ISSUER, None, Some("other-api"), false))
                .await
                .is_ok()
        );
    }

    /// A stand-in for Clerk's JWKS endpoint: serves the test key while
    /// `healthy`, a `503` otherwise, and counts every hit so the tests can see
    /// exactly when the verifier goes to the network.
    struct StubJwks {
        url: String,
        hits: Arc<std::sync::atomic::AtomicUsize>,
        healthy: Arc<std::sync::atomic::AtomicBool>,
    }

    impl StubJwks {
        async fn start() -> Self {
            use axum::{Router, extract::State, http::StatusCode, routing::get};
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

            let hits = Arc::new(AtomicUsize::new(0));
            let healthy = Arc::new(AtomicBool::new(false));
            let (n, e) = rsa_components();
            let document = serde_json::json!({
                "keys": [{ "kid": KID, "kty": "RSA", "alg": "RS256", "use": "sig", "n": n, "e": e }]
            });

            #[derive(Clone)]
            struct Stub {
                hits: Arc<AtomicUsize>,
                healthy: Arc<AtomicBool>,
                document: serde_json::Value,
            }

            async fn serve(
                State(stub): State<Stub>,
            ) -> (StatusCode, axum::Json<serde_json::Value>) {
                stub.hits.fetch_add(1, Ordering::SeqCst);
                if stub.healthy.load(Ordering::SeqCst) {
                    (StatusCode::OK, axum::Json(stub.document.clone()))
                } else {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        axum::Json(serde_json::json!({ "error": "down" })),
                    )
                }
            }

            let router = Router::new().route("/jwks", get(serve)).with_state(Stub {
                hits: hits.clone(),
                healthy: healthy.clone(),
                document,
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/jwks", listener.local_addr().unwrap());
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            Self { url, hits, healthy }
        }

        fn hits(&self) -> usize {
            self.hits.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn set_healthy(&self, healthy: bool) {
            self.healthy
                .store(healthy, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// A verifier that starts with an empty cache and refreshes from `url`.
    fn network_verifier(url: &str) -> AuthVerifier {
        AuthVerifier {
            jwks_url: Some(url.to_string()),
            issuer: ISSUER.to_string(),
            authorized_parties: vec![AZP.to_string()],
            audience: None,
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            keys: RwLock::new(HashMap::new()),
            refresh: Mutex::new(RefreshClock::default()),
        }
    }

    /// Rewind a stamp so the throttle it drives has expired, without sleeping.
    fn rewind(stamp: &mut Option<Instant>, by: Duration) {
        *stamp = Some(Instant::now().checked_sub(by).expect("recent Instant"));
    }

    #[tokio::test]
    async fn failed_jwks_fetch_backs_off_briefly_without_starting_refresh_interval() {
        let stub = StubJwks::start().await;
        let verifier = network_verifier(&stub.url);
        let good = token(ISSUER, AZP, false);

        // Clerk is down: the token cannot be verified and the failure is
        // remembered as a failure, not as a completed refresh.
        assert!(verifier.verify(&good).await.is_err());
        assert_eq!(stub.hits(), 1);
        {
            let clock = verifier.refresh.lock().await;
            assert!(clock.last_success.is_none());
            assert!(clock.last_failure.is_some());
        }

        // Inside the backoff the network is not hit again.
        assert!(verifier.verify(&good).await.is_err());
        assert_eq!(stub.hits(), 1);

        // Once the backoff lapses the next request retries -- well before the
        // 60s interval a success would have started.
        rewind(
            &mut verifier.refresh.lock().await.last_failure,
            JWKS_FAILURE_BACKOFF,
        );
        stub.set_healthy(true);
        assert_eq!(verifier.verify(&good).await.unwrap(), "user_123");
        assert_eq!(stub.hits(), 2);
        {
            let clock = verifier.refresh.lock().await;
            assert!(clock.last_success.is_some());
            assert!(clock.last_failure.is_none());
        }
    }

    #[tokio::test]
    async fn unknown_kid_refreshes_at_most_once_per_interval_after_success() {
        let stub = StubJwks::start().await;
        stub.set_healthy(true);
        let verifier = network_verifier(&stub.url);

        assert!(verifier.verify(&token(ISSUER, AZP, false)).await.is_ok());
        assert_eq!(stub.hits(), 1);

        // A bogus kid right after a successful refresh is answered from the
        // cache: rejected, no fetch.
        let bogus = token_with("not-a-real-kid", ISSUER, Some(AZP), None, false);
        assert!(verifier.verify(&bogus).await.is_err());
        assert_eq!(stub.hits(), 1);

        // After the interval the same request is allowed one more fetch.
        rewind(
            &mut verifier.refresh.lock().await.last_success,
            MIN_JWKS_REFRESH_INTERVAL,
        );
        assert!(verifier.verify(&bogus).await.is_err());
        assert_eq!(stub.hits(), 2);
    }
}
