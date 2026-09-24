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

/// Ceiling on how many Clerk instances one verifier trusts: production plus
/// the development instance the dev client signs in to, with room to spare.
/// Each instance costs its own JWKS cache and refresh throttle.
const MAX_CLERK_INSTANCES: usize = 4;

/// Verifies Clerk session tokens against Clerk's rotating RS256 public keys.
///
/// Several Clerk instances may be trusted (production, plus the development
/// instance a dev client signs in to). Each keeps its own key set, and a token
/// is only ever checked against the keys of the instance its `iss` names, so a
/// key from one instance can never vouch for a token claiming another.
pub struct AuthVerifier {
    instances: Vec<ClerkInstance>,
    authorized_parties: Vec<String>,
    client: reqwest::Client,
}

/// One trusted Clerk instance: its issuer, where its keys live, and the cache
/// of those keys.
struct ClerkInstance {
    issuer: String,
    jwks_url: Option<String>,
    /// Compared against `aud` (see [`AuthVerifier::verify`]). Only production
    /// takes `CLERK_AUDIENCE`.
    audience: Option<String>,
    keys: RwLock<HashMap<String, Arc<DecodingKey>>>,
    refresh: Mutex<RefreshClock>,
}

impl ClerkInstance {
    fn new(
        issuer: String,
        jwks_url: Option<String>,
        audience: Option<String>,
        keys: HashMap<String, Arc<DecodingKey>>,
    ) -> Self {
        Self {
            issuer,
            jwks_url,
            audience,
            keys: RwLock::new(keys),
            refresh: Mutex::new(RefreshClock::default()),
        }
    }
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

/// Why a token was refused. Every variant still answers a plain `401`; the
/// reason is only logged, so an operator can tell a JWKS outage from an
/// audience mismatch without guessing.
#[derive(Debug)]
enum Rejection {
    MalformedHeader,
    Algorithm,
    MissingKid,
    MalformedPayload,
    UntrustedIssuer,
    KeysUnavailable(JwksRefreshError),
    UnknownKid,
    /// Signature, expiry, not-before, issuer or audience, as `jsonwebtoken`
    /// reports it.
    Invalid(jsonwebtoken::errors::ErrorKind),
    Subject,
    /// The web origin claim was present but not an authorized party.
    AuthorizedParty(String),
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedHeader => write!(f, "header is not a JWT header"),
            Self::Algorithm => write!(f, "algorithm is not RS256"),
            Self::MissingKid => write!(f, "header has no kid"),
            Self::MalformedPayload => write!(f, "payload has no readable iss"),
            Self::UntrustedIssuer => write!(f, "issuer is not a trusted Clerk instance"),
            Self::KeysUnavailable(error) => write!(f, "JWKS unavailable: {error}"),
            Self::UnknownKid => write!(f, "kid is not in the issuer's JWKS"),
            Self::Invalid(kind) => write!(f, "claims or signature invalid: {kind:?}"),
            Self::Subject => write!(f, "empty sub or issuer mismatch"),
            Self::AuthorizedParty(azp) => write!(f, "azp {azp:?} is not an authorized party"),
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
        let authorized_parties = split_list(
            &authorized_parties
                .ok_or_else(|| anyhow::anyhow!("CLERK_AUTHORIZED_PARTIES is required"))?,
        );

        if authorized_parties.is_empty() {
            anyhow::bail!("CLERK_AUTHORIZED_PARTIES must contain at least one origin");
        }

        let mut instances = vec![ClerkInstance::new(
            issuer,
            Some(jwks_url),
            std::env::var("CLERK_AUDIENCE").ok(),
            HashMap::new(),
        )];
        let dev_issuers = std::env::var("CLERK_DEV_ISSUERS").unwrap_or_default();
        for dev_issuer in split_list(&dev_issuers) {
            instances.push(dev_instance(&dev_issuer)?);
        }
        if instances.len() > MAX_CLERK_INSTANCES {
            anyhow::bail!("at most {MAX_CLERK_INSTANCES} Clerk issuers may be trusted");
        }

        let client = reqwest::Client::builder()
            .timeout(JWKS_FETCH_TIMEOUT)
            .build()?;

        Ok(Some(Arc::new(Self {
            instances,
            authorized_parties,
            client,
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
        Self::from_rsa_pems(
            &[(kid, public_key_pem, issuer, audience)],
            authorized_parties,
        )
    }

    /// Creates a verifier trusting several fixed-key instances, one per
    /// `(kid, public key PEM, issuer, audience)`, with no network access.
    pub fn from_rsa_pems(
        instances: &[(&str, &[u8], &str, Option<String>)],
        authorized_parties: Vec<String>,
    ) -> anyhow::Result<Arc<Self>> {
        let mut trusted = Vec::with_capacity(instances.len());
        for (kid, public_key_pem, issuer, audience) in instances {
            let key = DecodingKey::from_rsa_pem(public_key_pem)?;
            trusted.push(ClerkInstance::new(
                (*issuer).to_owned(),
                None,
                audience.clone(),
                HashMap::from([((*kid).to_owned(), Arc::new(key))]),
            ));
        }
        Ok(Arc::new(Self {
            instances: trusted,
            authorized_parties,
            client: reqwest::Client::new(),
        }))
    }

    /// Re-fetch one instance's key set, throttled by its [`RefreshClock`]. The
    /// lock is held across the fetch so concurrent unknown-`kid` requests queue
    /// behind one network round trip instead of each starting their own.
    async fn refresh_keys(&self, instance: &ClerkInstance) -> Result<(), JwksRefreshError> {
        let url = instance
            .jwks_url
            .as_ref()
            .ok_or(JwksRefreshError::NotConfigured)?;
        let mut clock = instance.refresh.lock().await;
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
                *instance.keys.write().await = next_keys;
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
        self.check(token).await.map_err(|rejection| {
            // TODO: move to `tracing` once the crate adopts it. One line per
            // rejected token, naming the failed check and the claimed issuer
            // (public), never the token or its subject: without it every
            // failure below was an unexplained 401.
            eprintln!(
                "auth: rejected token ({rejection}); claimed iss {:?}",
                unverified_issuer(token)
            );
            AppError::Unauthorized
        })
    }

    async fn check(&self, token: &str) -> Result<String, Rejection> {
        let header = decode_header(token).map_err(|_| Rejection::MalformedHeader)?;
        if header.alg != Algorithm::RS256 {
            return Err(Rejection::Algorithm);
        }
        let kid = header.kid.ok_or(Rejection::MissingKid)?;

        // The unverified `iss` only picks which instance's keys to try; the
        // signature and the issuer are then both checked against that one
        // instance below, so a forged `iss` just selects keys that cannot
        // verify the token.
        let claimed_issuer = unverified_issuer(token).ok_or(Rejection::MalformedPayload)?;
        let instance = self
            .instances
            .iter()
            .find(|instance| instance.issuer == claimed_issuer)
            .ok_or(Rejection::UntrustedIssuer)?;

        let mut key = instance.keys.read().await.get(&kid).cloned();
        if key.is_none() {
            self.refresh_keys(instance)
                .await
                .map_err(Rejection::KeysUnavailable)?;
            key = instance.keys.read().await.get(&kid).cloned();
        }
        let key = key.ok_or(Rejection::UnknownKid)?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_required_spec_claims(&["exp", "iss", "sub"]);
        validation.set_issuer(&[instance.issuer.as_str()]);
        validation.validate_nbf = true;
        if let Some(audience) = instance.audience.as_deref() {
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
            .map_err(|error| Rejection::Invalid(error.into_kind()))?
            .claims;

        if claims.sub.trim().is_empty() || claims.iss != instance.issuer {
            return Err(Rejection::Subject);
        }
        // `azp` is Clerk's web origin claim: present on browser sessions (and
        // then it must be a listed party), absent on native app sessions.
        if let Some(azp) = claims.azp
            && !self
                .authorized_parties
                .iter()
                .any(|allowed| allowed == &azp)
        {
            return Err(Rejection::AuthorizedParty(azp));
        }

        Ok(claims.sub)
    }
}

/// Split a comma-separated environment list, dropping blanks.
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A Clerk development instance named in `CLERK_DEV_ISSUERS`, whose keys are
/// published at the issuer's standard JWKS path. Only `https://` issuers are
/// accepted, since the keys fetched from there decide who is signed in.
fn dev_instance(issuer: &str) -> anyhow::Result<ClerkInstance> {
    let issuer = issuer.trim_end_matches('/');
    if !issuer.starts_with("https://") {
        anyhow::bail!("CLERK_DEV_ISSUERS entries must be https:// URLs, got {issuer}");
    }
    let jwks_url = format!("{issuer}/.well-known/jwks.json");
    Ok(ClerkInstance::new(
        issuer.to_owned(),
        Some(jwks_url),
        None,
        HashMap::new(),
    ))
}

/// The `iss` claim of a token, read without verifying anything. Used only to
/// choose which trusted instance's keys to verify the token with.
fn unverified_issuer(token: &str) -> Option<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    #[derive(Deserialize)]
    struct IssuerOnly {
        iss: String,
    }

    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice::<IssuerOnly>(&bytes)
        .ok()
        .map(|claims| claims.iss)
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
            instances: vec![ClerkInstance::new(
                ISSUER.to_string(),
                Some(url.to_string()),
                None,
                HashMap::new(),
            )],
            authorized_parties: vec![AZP.to_string()],
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
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
            let clock = verifier.instances[0].refresh.lock().await;
            assert!(clock.last_success.is_none());
            assert!(clock.last_failure.is_some());
        }

        // Inside the backoff the network is not hit again.
        assert!(verifier.verify(&good).await.is_err());
        assert_eq!(stub.hits(), 1);

        // Once the backoff lapses the next request retries -- well before the
        // 60s interval a success would have started.
        rewind(
            &mut verifier.instances[0].refresh.lock().await.last_failure,
            JWKS_FAILURE_BACKOFF,
        );
        stub.set_healthy(true);
        assert_eq!(verifier.verify(&good).await.unwrap(), "user_123");
        assert_eq!(stub.hits(), 2);
        {
            let clock = verifier.instances[0].refresh.lock().await;
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
            &mut verifier.instances[0].refresh.lock().await.last_success,
            MIN_JWKS_REFRESH_INTERVAL,
        );
        assert!(verifier.verify(&bogus).await.is_err());
        assert_eq!(stub.hits(), 2);
    }

    const DEV_KID: &str = "meetcal-dev-key";
    const DEV_ISSUER: &str = "https://dev.clerk.test";

    /// A second keypair standing in for the Clerk development instance.
    static DEV_KEYS: LazyLock<TestKeys> = LazyLock::new(|| {
        let private = RsaPrivateKey::new(&mut OsRng, 2048).expect("generate dev RSA key");
        let public = RsaPublicKey::from(&private);
        TestKeys {
            private_pem: private
                .to_pkcs8_pem(LineEnding::LF)
                .expect("encode dev private key")
                .to_string(),
            public_pem: public
                .to_public_key_pem(LineEnding::LF)
                .expect("encode dev public key"),
        }
    });

    /// A native (no `azp`, no `aud`) token signed with `keys`.
    fn native_token_signed_by(keys: &TestKeys, kid: &str, issuer: &str) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        encode(
            &header,
            &Claims {
                sub: "user_dev",
                iss: issuer,
                azp: None,
                aud: None,
                exp: now + 300,
                nbf: now - 1,
            },
            &EncodingKey::from_rsa_pem(keys.private_pem.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn prod_and_dev_verifier(prod_audience: Option<&str>) -> Arc<AuthVerifier> {
        AuthVerifier::from_rsa_pems(
            &[
                (
                    KID,
                    TEST_KEYS.public_pem.as_bytes(),
                    ISSUER,
                    prod_audience.map(str::to_string),
                ),
                (DEV_KID, DEV_KEYS.public_pem.as_bytes(), DEV_ISSUER, None),
            ],
            vec![AZP.to_string()],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn a_trusted_dev_instance_token_verifies_alongside_production() {
        let verifier = prod_and_dev_verifier(None);
        assert_eq!(
            verifier
                .verify(&native_token_signed_by(&DEV_KEYS, DEV_KID, DEV_ISSUER))
                .await
                .unwrap(),
            "user_dev"
        );
        assert_eq!(
            verifier
                .verify(&token_with(KID, ISSUER, None, None, false))
                .await
                .unwrap(),
            "user_123"
        );
    }

    #[tokio::test]
    async fn dev_instance_tokens_are_rejected_unless_that_issuer_is_trusted() {
        assert!(
            verifier()
                .verify(&native_token_signed_by(&DEV_KEYS, DEV_KID, DEV_ISSUER))
                .await
                .is_err()
        );
    }

    /// Keys are bound to their instance: a dev key cannot sign for the
    /// production issuer, nor a production key for the dev issuer, even when
    /// the token names a `kid` the other instance knows.
    #[tokio::test]
    async fn one_instance_key_never_verifies_a_token_claiming_another() {
        let verifier = prod_and_dev_verifier(None);
        for kid in [DEV_KID, KID] {
            assert!(
                verifier
                    .verify(&native_token_signed_by(&DEV_KEYS, kid, ISSUER))
                    .await
                    .is_err(),
                "dev key signing as production (kid {kid})"
            );
            assert!(
                verifier
                    .verify(&native_token_signed_by(&TEST_KEYS, kid, DEV_ISSUER))
                    .await
                    .is_err(),
                "production key signing as dev (kid {kid})"
            );
        }
    }

    /// `CLERK_AUDIENCE` belongs to production; it does not reach the dev
    /// instance, whose native tokens carry no `aud`.
    #[tokio::test]
    async fn production_audience_does_not_apply_to_the_dev_instance() {
        let verifier = prod_and_dev_verifier(Some("meetcal-api"));
        assert_eq!(
            verifier
                .verify(&native_token_signed_by(&DEV_KEYS, DEV_KID, DEV_ISSUER))
                .await
                .unwrap(),
            "user_dev"
        );
        assert!(
            verifier
                .verify(&token_with(KID, ISSUER, None, None, false))
                .await
                .is_err(),
            "production still requires its audience"
        );
    }

    #[test]
    fn dev_issuers_must_be_https_and_map_to_their_jwks() {
        assert!(dev_instance("http://dev.clerk.test").is_err());
        let instance = dev_instance("https://dev.clerk.test/").unwrap();
        assert_eq!(instance.issuer, "https://dev.clerk.test");
        assert_eq!(
            instance.jwks_url.as_deref(),
            Some("https://dev.clerk.test/.well-known/jwks.json")
        );
        assert!(instance.audience.is_none());
    }

    #[test]
    fn unverified_issuer_reads_iss_and_rejects_malformed_tokens() {
        assert_eq!(
            unverified_issuer(&token(ISSUER, AZP, false)).as_deref(),
            Some(ISSUER)
        );
        assert_eq!(unverified_issuer(""), None);
        assert_eq!(unverified_issuer("only-one-part"), None);
        assert_eq!(unverified_issuer("a.!!!.c"), None);
        // `e30` is `{}`: valid JSON with no `iss`.
        assert_eq!(unverified_issuer("e30.e30.sig"), None);
    }

    /// Each refusal names the check that failed, so the log line tells an
    /// operator which one to fix.
    #[tokio::test]
    async fn rejections_name_the_failed_check() {
        use jsonwebtoken::errors::ErrorKind;

        let with_audience = verifier_with_audience(Some("meetcal-api"));
        assert!(matches!(
            with_audience
                .check(&token_with(KID, ISSUER, None, Some("convex"), false))
                .await,
            Err(Rejection::Invalid(ErrorKind::InvalidAudience))
        ));
        assert!(matches!(
            verifier().check(&token(ISSUER, AZP, true)).await,
            Err(Rejection::Invalid(ErrorKind::ExpiredSignature))
        ));
        assert!(matches!(
            verifier()
                .check(&token("https://wrong-issuer.test", AZP, false))
                .await,
            Err(Rejection::UntrustedIssuer)
        ));
        assert!(matches!(
            verifier()
                .check(&token(ISSUER, "https://evil.example", false))
                .await,
            Err(Rejection::AuthorizedParty(_))
        ));
        assert!(matches!(
            verifier()
                .check(&token_with("unknown-kid", ISSUER, None, None, false))
                .await,
            Err(Rejection::KeysUnavailable(JwksRefreshError::NotConfigured))
        ));
        assert!(matches!(
            verifier().check("not-a-jwt").await,
            Err(Rejection::MalformedHeader)
        ));
    }
}
