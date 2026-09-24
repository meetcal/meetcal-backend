//! Per-client rate limiting.
//!
//! Every request except [`HEALTH_PATH`] spends tokens from one bucket, chosen
//! by who is asking:
//!
//! - A request whose `X-MeetCal-Key` header matches a configured API key spends
//!   from that key's bucket ([`RateLimitSettings::key_burst`]). Keys are named
//!   secrets read only from `APP_RATE_LIMIT__KEYS` ([`ApiKeys::from_env`]);
//!   only their SHA-256 digests are kept, compared in constant time.
//! - Anything else, including an unknown or malformed key, is anonymous and
//!   spends from its client address's bucket ([`RateLimitSettings::ip_burst`]).
//!   A bad key is never answered differently from no key, so the API does not
//!   reveal whether a key exists.
//!
//! The client address is the TCP peer, unless the peer is a trusted proxy
//! ([`RateLimitSettings::trusted_proxies`], loopback by default) and
//! [`RateLimitSettings::trust_forwarded_for`] is on: then it is the rightmost
//! `X-Forwarded-For` entry, the one the proxy itself appended. IPv6 clients
//! are grouped by `/64`, the smallest block one subscriber is normally given.
//!
//! Routes cost tokens by how much database work they do ([`route_cost`]).
//! `/meets/package` is charged in two steps: [`PACKAGE_REVALIDATE_COST`]
//! before the handler runs, and the rest only when the answer is not a `304`
//! ([`route_settlement_cost`]), since a revalidation builds nothing.
//! Over the limit, an enforcing server answers `429 {"error":"rate limited"}`
//! with `Retry-After` in whole seconds until the bucket holds enough tokens
//! again. In shadow mode ([`RateLimitSettings::enforce`] off, the default) the
//! request goes through and a warning is logged instead, at most once per
//! client per [`LIMITED_LOG_INTERVAL`], so the limits can be checked against
//! real traffic before they are enforced.
use crate::AppError;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{
    DefaultDirectRateLimiter, DefaultKeyedRateLimiter, Quota, RateLimiter, clock::Clock,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    hash::{BuildHasher, RandomState},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    num::NonZeroU32,
    sync::Arc,
    time::Duration,
};
use subtle::{ConditionallySelectable, ConstantTimeEq};
use tokio::sync::Notify;

/// Header a keyed client sends its secret in, lowercase as `http` stores it.
pub const API_KEY_HEADER: &str = "x-meetcal-key";

/// Environment variable holding the API keys, as `name:secret` pairs separated
/// by commas. Read directly, never through `configuration.yaml`, so a secret
/// cannot end up in a committed config file.
pub const API_KEYS_ENV: &str = "APP_RATE_LIMIT__KEYS";

/// The only path that never spends tokens: deploys and uptime monitors poll it,
/// and it must answer while a client (or the proxy's own address) is limited.
pub const HEALTH_PATH: &str = "/health";

// ---------------------------------------------------------------------------
// Route costs
// ---------------------------------------------------------------------------

/// Cost of a request to any route not listed below: an indexed lookup or a
/// small reference list, usually well under a millisecond of database time.
pub const DEFAULT_ROUTE_COST: u32 = 1;
/// `/meets/package` bundles a whole meet (start list, schedule, history). It
/// is cached server-side and revalidated with `If-None-Match`, so most calls
/// are cheap, but a miss is the most expensive build the API does. The app
/// calls it once per sync, in place of a dozen smaller requests.
pub const PACKAGE_ROUTE_COST: u32 = 4;
/// What a `/meets/package` request spends before its handler runs, and all a
/// `304 Not Modified` ever costs: the freshness stamp lookup and an ETag
/// compare, the same indexed work as any default route. A response with a
/// body settles the remaining `PACKAGE_ROUTE_COST - PACKAGE_REVALIDATE_COST`
/// afterwards ([`route_settlement_cost`]).
pub const PACKAGE_REVALIDATE_COST: u32 = DEFAULT_ROUTE_COST;
/// The package path, matched exactly like every other route cost.
pub const PACKAGE_PATH: &str = "/meets/package";
/// `/search` scans `lifting_results` by name and date range.
pub const SEARCH_ROUTE_COST: u32 = 5;
/// `/lifting-results/by-names`, `/recent` and `/bests`: up to
/// `MAX_NAME_LIST_LEN` names per request, each a separate history. The CSV
/// `GET` and the JSON `POST` run the same query, so both methods cost the
/// same; weighting only `POST` would just move heavy callers to `GET`.
pub const NAME_LIST_ROUTE_COST: u32 = 5;
/// `/clubs/meet-stats` aggregates every result for a club's athletes.
pub const MEET_STATS_ROUTE_COST: u32 = 5;
/// The most any one request costs. Every bucket must hold at least this many
/// tokens, or that route could never be served.
pub const MAX_ROUTE_COST: u32 = max_u32(
    max_u32(DEFAULT_ROUTE_COST, PACKAGE_ROUTE_COST),
    max_u32(
        max_u32(SEARCH_ROUTE_COST, NAME_LIST_ROUTE_COST),
        MEET_STATS_ROUTE_COST,
    ),
);

const fn max_u32(a: u32, b: u32) -> u32 {
    if a > b { a } else { b }
}

const _: () = assert!(
    PACKAGE_REVALIDATE_COST >= 1 && PACKAGE_REVALIDATE_COST <= PACKAGE_ROUTE_COST,
    "a package revalidation spends something, and never more than a build"
);

/// Tokens one request to `path` spends in total. Matched on the exact path
/// (axum routes are exact), so an unknown path, a 404, costs
/// [`DEFAULT_ROUTE_COST`]. `/meets/package` spends this much only when it
/// answers with a body; see [`route_upfront_cost`] / [`route_settlement_cost`].
pub fn route_cost(path: &str) -> u32 {
    match path {
        PACKAGE_PATH => PACKAGE_ROUTE_COST,
        "/search" => SEARCH_ROUTE_COST,
        "/lifting-results/by-names" | "/lifting-results/recent" | "/lifting-results/bests" => {
            NAME_LIST_ROUTE_COST
        }
        "/clubs/meet-stats" => MEET_STATS_ROUTE_COST,
        _ => DEFAULT_ROUTE_COST,
    }
}

/// Tokens spent before the handler runs. The whole [`route_cost`] for every
/// route but `/meets/package`, which pays [`PACKAGE_REVALIDATE_COST`] now and
/// the rest once the middleware can see whether it built anything.
pub fn route_upfront_cost(path: &str) -> u32 {
    if path == PACKAGE_PATH {
        PACKAGE_REVALIDATE_COST
    } else {
        route_cost(path)
    }
}

/// Tokens spent after the handler answered with `status`: the rest of the
/// package cost for anything but a `304`, which built nothing. Zero for
/// every other route, whose whole cost was spent up front.
pub fn route_settlement_cost(path: &str, status: StatusCode) -> u32 {
    if path == PACKAGE_PATH && status != StatusCode::NOT_MODIFIED {
        PACKAGE_ROUTE_COST - PACKAGE_REVALIDATE_COST
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Default budgets and the traffic they are sized for
// ---------------------------------------------------------------------------

/// Anonymous bucket refill rate, per client address (IPv4 or IPv6 `/64`).
///
/// Sized for a meet venue: hundreds of phones share one Wi-Fi or carrier NAT
/// address. The app syncs every five minutes with `/meets` plus a
/// `/meets/package` revalidation ([`APP_SYNC_COST`] tokens), so
/// [`VENUE_DEVICES`] phones need about 8.3 tokens/s; 40 leaves four times that
/// ([`VENUE_HEADROOM`]) for people opening meets, schedules and results
/// between syncs. One host looping on the API is held to 40 cheap requests a
/// second, or 8 searches, once its burst is spent.
pub const DEFAULT_IP_TOKENS_PER_SECOND: u32 = 40;
/// Anonymous bucket size: the most one client address can spend at once.
///
/// 1,200 tokens is 30 seconds of refill. It absorbs a venue where hundreds of
/// phones open the app within the same minute (a session starts, the Wi-Fi
/// comes back), and a meetcal-cli run of [`CLI_BURST_REQUESTS`] name-list
/// batches back to back ([`CLI_BURST_REQUESTS`] x [`NAME_LIST_ROUTE_COST`] =
/// 200 tokens) six times over. A host sending 500 requests a second exhausts
/// it in under three seconds.
pub const DEFAULT_IP_BURST: u32 = 1_200;
/// Keyed bucket refill rate. A keyed client is a server acting for many users
/// (atlas-rn on Convex), so it gets five times an anonymous address.
pub const DEFAULT_KEY_TOKENS_PER_SECOND: u32 = 200;
/// Keyed bucket size: 30 seconds of refill, like the anonymous bucket.
pub const DEFAULT_KEY_BURST: u32 = 6_000;

/// App devices behind one venue address the defaults must carry.
pub const VENUE_DEVICES: u32 = 500;
/// Tokens one app sync spends: `/meets` plus `/meets/package`.
pub const APP_SYNC_COST: u32 = DEFAULT_ROUTE_COST + PACKAGE_ROUTE_COST;
/// Seconds between app syncs (the app's `SYNC_INTERVAL`, five minutes).
pub const APP_SYNC_INTERVAL_SECS: u32 = 5 * 60;
/// How many times the venue's background sync load the refill rate covers.
pub const VENUE_HEADROOM: u32 = 4;
/// Sequential requests one meetcal-cli command makes in a few seconds.
pub const CLI_BURST_REQUESTS: u32 = 40;

const _: () = assert!(
    VENUE_DEVICES * APP_SYNC_COST * VENUE_HEADROOM
        <= DEFAULT_IP_TOKENS_PER_SECOND * APP_SYNC_INTERVAL_SECS,
    "the default anonymous rate must carry a venue of app devices with headroom"
);

// The background sync above is the steady state. The expensive moment is the
// first open of a downloaded meet: the app revalidates the package and then
// refreshes every rostered lifter's history through `/lifting-results/by-names`
// in chunks of `APP_NAME_LIST_CHUNK`, each chunk costing the full
// `NAME_LIST_ROUTE_COST` whatever its size, so one phone spends
// `HISTORY_REFRESH_COST` (194 tokens for a 1,500-lifter roster) in a burst.
// Sorting a roster by best is the same order of cost (75 to 150 tokens).
//
// Spread over the first hour of a meet day that fits the defaults, which the
// assertion below pins. It does NOT fit the first minute: 200 phones opening
// the app as doors open need 38,800 tokens against 1,200 burst + 2,400 of
// refill, so enforcing the current `DEFAULT_IP_BURST` would 429 a venue on
// first open. `venue_first_minute_shortfall` keeps that number honest for
// docs/rate-limits.md, which says to raise the anonymous burst toward the
// keyed one before flipping `APP_RATE_LIMIT__ENFORCE`.

/// Lifters on the largest roster the app downloads (a national championship).
pub const VENUE_ROSTER_ATHLETES: u32 = 1_500;
/// Names the app puts in one `/lifting-results/by-names` request (its chunk
/// size; the server accepts up to `MAX_NAME_LIST_LEN`).
pub const APP_NAME_LIST_CHUNK: u32 = 40;
/// Tokens one phone spends refreshing a downloaded meet's history: the
/// package revalidation plus one name-list request per roster chunk.
pub const HISTORY_REFRESH_COST: u32 =
    PACKAGE_ROUTE_COST + VENUE_ROSTER_ATHLETES.div_ceil(APP_NAME_LIST_CHUNK) * NAME_LIST_ROUTE_COST;
/// Window over which a venue's first-open refreshes are spread for sizing.
pub const VENUE_FIRST_HOUR_SECS: u32 = 60 * 60;
/// Phones that open the app in the same minute as a session starts.
pub const VENUE_FIRST_MINUTE_DEVICES: u32 = 200;

const _: () = assert!(
    VENUE_DEVICES * HISTORY_REFRESH_COST
        <= DEFAULT_IP_BURST + DEFAULT_IP_TOKENS_PER_SECOND * VENUE_FIRST_HOUR_SECS,
    "the default anonymous budget must carry every venue device refreshing a downloaded \
     meet's history within the first hour"
);

/// Tokens `devices` phones refreshing history in the same minute would need
/// beyond what one anonymous bucket holds plus a minute of refill; zero when
/// the defaults already cover them. The first-open gap the docs quote.
pub const fn venue_first_minute_shortfall(devices: u32) -> u32 {
    (devices * HISTORY_REFRESH_COST)
        .saturating_sub(DEFAULT_IP_BURST + DEFAULT_IP_TOKENS_PER_SECOND * 60)
}
const _: () = assert!(
    CLI_BURST_REQUESTS * MAX_ROUTE_COST <= DEFAULT_IP_BURST,
    "the default anonymous burst must fit a meetcal-cli run of the costliest route"
);
const _: () = assert!(DEFAULT_IP_BURST >= MAX_ROUTE_COST && DEFAULT_KEY_BURST >= MAX_ROUTE_COST);
const _: () = assert!(DEFAULT_KEY_TOKENS_PER_SECOND >= DEFAULT_IP_TOKENS_PER_SECOND);

// ---------------------------------------------------------------------------
// Housekeeping and logging bounds
// ---------------------------------------------------------------------------

/// How often idle client buckets are dropped. A bucket is idle once it has
/// refilled completely (at most `burst / rate`, 30s with the defaults), so
/// the map holds only clients seen within roughly the last 90 seconds.
pub const CLIENT_EVICTION_INTERVAL: Duration = Duration::from_secs(60);
/// Most client addresses given their own bucket. At this size a sweep runs at
/// once instead of waiting for [`CLIENT_EVICTION_INTERVAL`], and until it
/// frees room every address is charged to one of [`OVERFLOW_BUCKETS`] shared
/// buckets instead (see [`RateLimit::charged_client`]), so an address spray
/// cannot grow the map past `MAX_TRACKED_CLIENTS + OVERFLOW_BUCKETS`. About
/// 64 MB of map at this size.
pub const MAX_TRACKED_CLIENTS: usize = 1_000_000;
/// Shared buckets for addresses that arrive while the map is full. Slots are
/// picked by the per-process random hash key, so a sprayer cannot aim at the
/// slot a given user lands in; each slot has the normal per-address quota.
pub const OVERFLOW_BUCKETS: u16 = 1024;
/// Overflow slots live in the IPv6 discard-only prefix `100::/64` (RFC 6666),
/// which no real client address (or `/64` client network) can fall in.
const OVERFLOW_PREFIX: u16 = 0x0100;
/// Shortest gap between two sweeps, so a sustained spray costs at most one
/// full-map pass per second.
pub const MIN_EVICTION_GAP: Duration = Duration::from_secs(1);
/// At most one "rate limited" warning per client per interval, so a client
/// hammering the API cannot flood the log.
pub const LIMITED_LOG_INTERVAL: Duration = Duration::from_secs(60);
/// At most one "X-Forwarded-For from an untrusted peer" warning per interval.
pub const UNTRUSTED_FORWARD_LOG_INTERVAL: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// API key bounds
// ---------------------------------------------------------------------------

/// Most API keys one server accepts. Each is a separate bucket held forever.
pub const MAX_API_KEYS: usize = 32;
/// Longest key name (`[a-z0-9_-]`), which appears in logs as `key:<name>`.
pub const MAX_API_KEY_NAME_LEN: usize = 32;
/// Shortest accepted secret: 32 characters, e.g. `openssl rand -hex 32` (64).
pub const MIN_API_KEY_SECRET_LEN: usize = 32;
/// Longest accepted secret. A longer header is not hashed at all.
pub const MAX_API_KEY_SECRET_LEN: usize = 256;
/// Most trusted proxy entries (`trusted_proxies`).
pub const MAX_TRUSTED_PROXIES: usize = 16;

/// Proxies trusted by default: the loopback addresses, where Caddy connects
/// from when both run on the host.
pub const DEFAULT_TRUSTED_PROXIES: &str = "127.0.0.0/8,::1/128";

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// The `rate_limit` section of the configuration, overridable per field with
/// `APP_RATE_LIMIT__<FIELD>` (e.g. `APP_RATE_LIMIT__ENFORCE=true`). Every field
/// has a default, so the section can be absent. The API keys are not here:
/// they are secrets and come only from [`API_KEYS_ENV`].
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RateLimitSettings {
    /// Answer `429` past the limit. Off by default: shadow mode logs a
    /// would-be rejection and serves the request anyway.
    pub enforce: bool,
    /// Take the client address from `X-Forwarded-For` when the peer is one of
    /// [`Self::trusted_proxies`]. On by default: production sits behind Caddy.
    pub trust_forwarded_for: bool,
    /// Comma-separated addresses or CIDR blocks whose `X-Forwarded-For` is
    /// trusted. Only list proxies that overwrite the header with the address
    /// they accepted the connection from, as Caddy does.
    pub trusted_proxies: String,
    /// Anonymous refill rate, tokens per second per client address.
    pub ip_tokens_per_second: u32,
    /// Anonymous bucket size.
    pub ip_burst: u32,
    /// Keyed refill rate, tokens per second per key.
    pub key_tokens_per_second: u32,
    /// Keyed bucket size.
    pub key_burst: u32,
    /// Requests in flight server-wide before new ones are shed with `503`.
    /// Always applies, whatever [`Self::enforce`] says; see
    /// [`crate::common::load_shed`].
    pub max_in_flight: usize,
}

impl Default for RateLimitSettings {
    fn default() -> Self {
        Self {
            enforce: false,
            trust_forwarded_for: true,
            trusted_proxies: DEFAULT_TRUSTED_PROXIES.to_string(),
            ip_tokens_per_second: DEFAULT_IP_TOKENS_PER_SECOND,
            ip_burst: DEFAULT_IP_BURST,
            key_tokens_per_second: DEFAULT_KEY_TOKENS_PER_SECOND,
            key_burst: DEFAULT_KEY_BURST,
            max_in_flight: crate::DEFAULT_MAX_IN_FLIGHT,
        }
    }
}

fn quota(field: &str, per_second: u32, burst: u32) -> anyhow::Result<Quota> {
    let per_second = NonZeroU32::new(per_second).ok_or_else(|| {
        anyhow::anyhow!("rate_limit.{field}_tokens_per_second must be at least 1")
    })?;
    let burst = NonZeroU32::new(burst)
        .filter(|burst| burst.get() >= MAX_ROUTE_COST)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "rate_limit.{field}_burst must be at least {MAX_ROUTE_COST}, the costliest route"
            )
        })?;
    Ok(Quota::per_second(per_second).allow_burst(burst))
}

// ---------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------

/// Configured API keys: a name for logs and the SHA-256 of the secret. The
/// secret itself is dropped once hashed.
pub struct ApiKeys {
    keys: Vec<ApiKey>,
}

struct ApiKey {
    name: String,
    digest: [u8; 32],
}

impl std::fmt::Debug for ApiKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.keys.iter().map(|key| &key.name))
            .finish()
    }
}

impl ApiKeys {
    /// No keys: every client is anonymous.
    pub fn none() -> Self {
        Self { keys: Vec::new() }
    }

    /// Reads [`API_KEYS_ENV`]. Unset or blank means no keys; anything
    /// malformed is an error, so a typo stops startup instead of silently
    /// demoting a partner to the anonymous budget.
    pub fn from_env() -> anyhow::Result<Self> {
        match std::env::var(API_KEYS_ENV) {
            Ok(spec) => Self::parse(&spec),
            Err(std::env::VarError::NotPresent) => Ok(Self::none()),
            Err(error) => Err(anyhow::anyhow!("{API_KEYS_ENV}: {error}")),
        }
    }

    /// Parses `name:secret[,name:secret...]`. Names are `[a-z0-9_-]`, at most
    /// [`MAX_API_KEY_NAME_LEN`]; secrets are `[A-Za-z0-9_-]`, between
    /// [`MIN_API_KEY_SECRET_LEN`] and [`MAX_API_KEY_SECRET_LEN`]. Names and
    /// secrets must be unique. Errors never echo a secret.
    pub fn parse(spec: &str) -> anyhow::Result<Self> {
        let mut keys: Vec<ApiKey> = Vec::new();
        for (index, entry) in spec.split(',').enumerate() {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            anyhow::ensure!(
                keys.len() < MAX_API_KEYS,
                "{API_KEYS_ENV}: more than {MAX_API_KEYS} keys"
            );
            let Some((name, secret)) = entry.split_once(':') else {
                anyhow::bail!("{API_KEYS_ENV}: entry {} is not name:secret", index + 1);
            };
            let name = name.trim();
            anyhow::ensure!(
                !name.is_empty()
                    && name.len() <= MAX_API_KEY_NAME_LEN
                    && name.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || byte == b'-'
                            || byte == b'_'
                    }),
                "{API_KEYS_ENV}: entry {} has an invalid name (use 1-{MAX_API_KEY_NAME_LEN} of a-z 0-9 - _)",
                index + 1
            );
            anyhow::ensure!(
                (MIN_API_KEY_SECRET_LEN..=MAX_API_KEY_SECRET_LEN).contains(&secret.len())
                    && secret
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
                "{API_KEYS_ENV}: key {name:?} needs a secret of {MIN_API_KEY_SECRET_LEN}-{MAX_API_KEY_SECRET_LEN} characters from A-Z a-z 0-9 - _"
            );
            let digest: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
            anyhow::ensure!(
                keys.iter().all(|key| key.name != name),
                "{API_KEYS_ENV}: key name {name:?} is listed twice"
            );
            anyhow::ensure!(
                keys.iter().all(|key| key.digest != digest),
                "{API_KEYS_ENV}: key {name:?} reuses another key's secret"
            );
            keys.push(ApiKey {
                name: name.to_string(),
                digest,
            });
        }
        Ok(Self { keys })
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        self.keys.iter().map(|key| key.name.as_str()).collect()
    }

    /// The index of the key whose secret is `presented`, if any. Compares the
    /// digest against every key in constant time, so the timing reveals
    /// neither whether a key matched nor which one.
    fn find(&self, presented: &[u8]) -> Option<usize> {
        if self.keys.is_empty()
            || !(MIN_API_KEY_SECRET_LEN..=MAX_API_KEY_SECRET_LEN).contains(&presented.len())
        {
            return None;
        }
        let digest: [u8; 32] = Sha256::digest(presented).into();
        // 0 means no match; key `i` is stored as `i + 1`.
        let mut found = 0u32;
        for (index, key) in self.keys.iter().enumerate() {
            let candidate = u32::try_from(index + 1).unwrap_or(u32::MAX);
            found = u32::conditional_select(&found, &candidate, key.digest.ct_eq(&digest));
        }
        (found as usize).checked_sub(1)
    }
}

// ---------------------------------------------------------------------------
// Client identity
// ---------------------------------------------------------------------------

/// Who a request is charged to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Client {
    /// A configured API key, by index into [`ApiKeys`].
    Key(usize),
    /// An anonymous client address: IPv4, or an IPv6 `/64` network.
    Ip(IpAddr),
}

/// An address or CIDR block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    network: IpAddr,
    prefix: u8,
}

impl IpNet {
    /// Parses `addr` or `addr/prefix`. An IPv4-mapped IPv6 address is read as
    /// the IPv4 address it carries, as peers are.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let (addr, prefix) = match raw.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix.parse::<u8>().ok()?)),
            None => (raw, None),
        };
        let addr = addr.parse::<IpAddr>().ok()?.to_canonical();
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(max);
        if prefix > max {
            return None;
        }
        Some(Self {
            network: mask(addr, prefix),
            prefix,
        })
    }

    pub fn contains(&self, addr: IpAddr) -> bool {
        let addr = addr.to_canonical();
        addr.is_ipv4() == self.network.is_ipv4() && mask(addr, self.prefix) == self.network
    }
}

impl std::fmt::Display for IpNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

/// `addr` with every bit past `prefix` cleared.
fn mask(addr: IpAddr, prefix: u8) -> IpAddr {
    match addr {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            let kept = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V4(Ipv4Addr::from(bits & kept))
        }
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            let kept = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V6(Ipv6Addr::from(bits & kept))
        }
    }
}

/// Prefix one IPv6 subscriber usually controls in full; addresses inside it
/// share one bucket, or a single host could rotate through 2^64 addresses.
pub const IPV6_CLIENT_PREFIX: u8 = 64;

/// The bucket key for a client address: IPv4 as is (an IPv4-mapped IPv6
/// address counts as its IPv4 address), IPv6 cut to its `/64`.
pub fn client_network(addr: IpAddr) -> IpAddr {
    match addr.to_canonical() {
        v4 @ IpAddr::V4(_) => v4,
        v6 @ IpAddr::V6(_) => mask(v6, IPV6_CLIENT_PREFIX),
    }
}

/// The rightmost `X-Forwarded-For` entry: the address the nearest proxy
/// accepted the connection from. Entries to its left were sent by the client
/// and can be anything. `None` when absent or not a bare IP address.
pub fn rightmost_forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    let last_header = headers.get_all("x-forwarded-for").iter().next_back()?;
    let last_entry = last_header.to_str().ok()?.rsplit(',').next()?.trim();
    last_entry.parse::<IpAddr>().ok()
}

// ---------------------------------------------------------------------------
// The limiter
// ---------------------------------------------------------------------------

/// Outcome of charging a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Over the limit; the bucket holds the request's cost again after this
    /// many whole seconds (at least 1).
    Limited {
        retry_after_secs: u64,
    },
}

pub struct RateLimit {
    enforce: bool,
    trust_forwarded_for: bool,
    trusted_proxies: Vec<IpNet>,
    keys: ApiKeys,
    key_buckets: Vec<DefaultDirectRateLimiter>,
    ip_buckets: DefaultKeyedRateLimiter<IpAddr>,
    /// The validated settings, kept for the startup log line.
    settings: RateLimitSettings,
    /// One token per client per [`LIMITED_LOG_INTERVAL`]: gates the warning.
    limited_log: DefaultKeyedRateLimiter<Client>,
    untrusted_forward_log: DefaultDirectRateLimiter,
    /// Keys the client tag hash. Random per process, so a tag cannot be
    /// reversed by hashing all 2^32 IPv4 addresses.
    tag_key: RandomState,
    evict_now: Notify,
    /// [`MAX_TRACKED_CLIENTS`], lowered in tests.
    max_tracked: usize,
}

impl std::fmt::Debug for RateLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimit")
            .field("enforce", &self.enforce)
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

impl RateLimit {
    /// Validates `settings` and builds the buckets. Any invalid setting is an
    /// error: the server refuses to start rather than run with limits nobody
    /// chose.
    pub fn new(settings: &RateLimitSettings, keys: ApiKeys) -> anyhow::Result<Self> {
        let ip_quota = quota("ip", settings.ip_tokens_per_second, settings.ip_burst)?;
        let key_quota = quota("key", settings.key_tokens_per_second, settings.key_burst)?;
        let mut trusted_proxies = Vec::new();
        for entry in settings.trusted_proxies.split(',') {
            if entry.trim().is_empty() {
                continue;
            }
            anyhow::ensure!(
                trusted_proxies.len() < MAX_TRUSTED_PROXIES,
                "rate_limit.trusted_proxies lists more than {MAX_TRUSTED_PROXIES} entries"
            );
            let net = IpNet::parse(entry).ok_or_else(|| {
                anyhow::anyhow!("rate_limit.trusted_proxies: {entry:?} is not an address or CIDR")
            })?;
            trusted_proxies.push(net);
        }
        let key_buckets = (0..keys.len())
            .map(|_| RateLimiter::direct(key_quota))
            .collect();
        let log_quota = Quota::with_period(LIMITED_LOG_INTERVAL)
            .ok_or_else(|| anyhow::anyhow!("LIMITED_LOG_INTERVAL must be non-zero"))?;
        let untrusted_quota = Quota::with_period(UNTRUSTED_FORWARD_LOG_INTERVAL)
            .ok_or_else(|| anyhow::anyhow!("UNTRUSTED_FORWARD_LOG_INTERVAL must be non-zero"))?;
        Ok(Self {
            enforce: settings.enforce,
            trust_forwarded_for: settings.trust_forwarded_for,
            trusted_proxies,
            keys,
            key_buckets,
            ip_buckets: RateLimiter::keyed(ip_quota),
            settings: settings.clone(),
            limited_log: RateLimiter::keyed(log_quota),
            untrusted_forward_log: RateLimiter::direct(untrusted_quota),
            tag_key: RandomState::new(),
            evict_now: Notify::new(),
            max_tracked: MAX_TRACKED_CLIENTS,
        })
    }

    #[cfg(test)]
    fn with_max_tracked(mut self, max_tracked: usize) -> Self {
        self.max_tracked = max_tracked;
        self
    }

    /// The client a request is actually charged to (and logged as). While the
    /// address map is at [`MAX_TRACKED_CLIENTS`], an address is folded into
    /// one of [`OVERFLOW_BUCKETS`] shared slots instead of getting a new
    /// bucket, and a sweep is requested. The keyed store cannot tell a new
    /// address from a tracked one, so tracked addresses fold too until the
    /// sweep frees room: fairness degrades during a spray, memory does not.
    pub fn charged_client(&self, client: Client) -> Client {
        match client {
            Client::Ip(addr) if self.ip_buckets.len() >= self.max_tracked => {
                self.evict_now.notify_one();
                let slot = (self.tag_key.hash_one(addr) % u64::from(OVERFLOW_BUCKETS)) as u16;
                Client::Ip(IpAddr::V6(std::net::Ipv6Addr::new(
                    OVERFLOW_PREFIX,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    slot,
                )))
            }
            other => other,
        }
    }

    pub fn enforce(&self) -> bool {
        self.enforce
    }

    /// One startup line with the effective limits (key names, never secrets),
    /// so an operator can confirm what the environment actually set.
    pub fn log_settings(&self) {
        let proxies: Vec<String> = self
            .trusted_proxies
            .iter()
            .map(|net| net.to_string())
            .collect();
        tracing::info!(
            enforce = self.enforce,
            trust_forwarded_for = self.trust_forwarded_for,
            trusted_proxies = %proxies.join(","),
            ip_tokens_per_second = self.settings.ip_tokens_per_second,
            ip_burst = self.settings.ip_burst,
            key_tokens_per_second = self.settings.key_tokens_per_second,
            key_burst = self.settings.key_burst,
            max_in_flight = self.settings.max_in_flight,
            api_keys = %self.keys.names().join(","),
            "rate limiting configured"
        );
    }

    fn is_trusted_proxy(&self, peer: IpAddr) -> bool {
        self.trusted_proxies.iter().any(|net| net.contains(peer))
    }

    /// Who the request is charged to. `peer` is `None` only when the server
    /// was not built with connect info; all such requests share one bucket.
    pub fn identify(&self, peer: Option<SocketAddr>, headers: &HeaderMap) -> Client {
        if let Some(index) = headers
            .get(API_KEY_HEADER)
            .and_then(|value| self.keys.find(value.as_bytes()))
        {
            return Client::Key(index);
        }
        let peer = peer.map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |peer| {
            peer.ip().to_canonical()
        });
        if self.trust_forwarded_for && headers.contains_key("x-forwarded-for") {
            if self.is_trusted_proxy(peer) {
                if let Some(forwarded) = rightmost_forwarded_for(headers) {
                    return Client::Ip(client_network(forwarded));
                }
            } else if self.untrusted_forward_log.check().is_ok() {
                tracing::warn!(
                    peer = %self.describe_peer(peer),
                    "ignored X-Forwarded-For from a peer outside rate_limit.trusted_proxies; \
                     if the API sits behind a proxy on this address, add it there"
                );
            }
        }
        Client::Ip(client_network(peer))
    }

    /// A private or loopback peer is a proxy or container gateway, safe to log
    /// as is (and what an operator needs to configure `trusted_proxies`); a
    /// public one may be a user and is logged as its tag.
    fn describe_peer(&self, peer: IpAddr) -> String {
        let internal = match peer {
            IpAddr::V4(v4) => v4.is_private() || v4.is_loopback(),
            IpAddr::V6(v6) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
        };
        if internal {
            peer.to_string()
        } else {
            format!("tag:{}", self.tag(&client_network(peer)))
        }
    }

    /// Spends `cost` tokens from `client`'s bucket.
    pub fn check(&self, client: &Client, cost: u32) -> Decision {
        let Some(cost) = NonZeroU32::new(cost) else {
            return Decision::Allow;
        };
        let (outcome, now) = match client {
            Client::Key(index) => match self.key_buckets.get(*index) {
                Some(bucket) => (bucket.check_n(cost), bucket.clock().now()),
                // Unreachable: indices come from `ApiKeys::find`. Fail closed.
                None => {
                    return Decision::Limited {
                        retry_after_secs: 1,
                    };
                }
            },
            // Callers fold the address first (`charged_client`), so this never
            // grows the map past the cap plus the overflow slots.
            Client::Ip(addr) => (
                self.ip_buckets.check_key_n(addr, cost),
                self.ip_buckets.clock().now(),
            ),
        };
        match outcome {
            Ok(Ok(())) => Decision::Allow,
            Ok(Err(not_until)) => Decision::Limited {
                retry_after_secs: whole_seconds_at_least_one(not_until.wait_time_from(now)),
            },
            // The cost exceeds the whole bucket. `new` rejects such settings,
            // so this is unreachable; fail closed.
            Err(_insufficient) => Decision::Limited {
                retry_after_secs: 1,
            },
        }
    }

    /// Bucket kind for logs: `ip` or `key:<name>`.
    fn kind(&self, client: &Client) -> String {
        match client {
            Client::Key(index) => match self.keys.keys.get(*index) {
                Some(key) => format!("key:{}", key.name),
                None => "key:?".to_string(),
            },
            Client::Ip(IpAddr::V6(v6)) if v6.segments()[0] == OVERFLOW_PREFIX => {
                "ip-overflow".to_string()
            }
            Client::Ip(_) => "ip".to_string(),
        }
    }

    /// Short, non-reversible label for a client address: 32 bits of a keyed
    /// SipHash. Stable for the life of the process, so one client's warnings
    /// can be followed across intervals without logging its address.
    fn tag(&self, addr: &IpAddr) -> String {
        format!("{:08x}", self.tag_key.hash_one(addr) >> 32)
    }

    /// Logs one warning for a limited request, at most once per client per
    /// [`LIMITED_LOG_INTERVAL`]. Only the path is logged: the query string can
    /// carry athlete names.
    pub fn log_limited(&self, client: &Client, path: &str, retry_after_secs: u64) {
        if self.limited_log.check_key(client).is_err() {
            return;
        }
        let tag = match client {
            Client::Ip(addr) => self.tag(addr),
            Client::Key(_) => "-".to_string(),
        };
        tracing::warn!(
            bucket = %self.kind(client),
            client_tag = %tag,
            path = %path,
            retry_after_secs,
            enforced = self.enforce,
            "rate limit exceeded"
        );
    }

    /// Drops buckets that have refilled completely (indistinguishable from a
    /// new client) and the log gates that have expired.
    pub fn evict_idle(&self) {
        self.ip_buckets.retain_recent();
        self.ip_buckets.shrink_to_fit();
        self.limited_log.retain_recent();
        self.limited_log.shrink_to_fit();
        let tracked = self.ip_buckets.len();
        if tracked >= self.max_tracked {
            tracing::warn!(
                tracked,
                cap = self.max_tracked,
                "client address map still full after eviction; new addresses share overflow buckets"
            );
        }
    }

    /// Tracked client addresses (approximate under concurrent updates).
    pub fn tracked_clients(&self) -> usize {
        self.ip_buckets.len()
    }

    /// Sweeps idle clients every [`CLIENT_EVICTION_INTERVAL`], or sooner when
    /// [`MAX_TRACKED_CLIENTS`] is passed, never more often than
    /// [`MIN_EVICTION_GAP`]. Runs for the life of the server; `run_with_auth`
    /// aborts it on shutdown.
    pub async fn evict_idle_forever(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(CLIENT_EVICTION_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = self.evict_now.notified() => {}
            }
            self.evict_idle();
            tokio::time::sleep(MIN_EVICTION_GAP).await;
        }
    }
}

/// `wait` rounded up to whole seconds, at least 1: `Retry-After` cannot
/// express a fraction, and `0` would invite an immediate retry that fails.
pub fn whole_seconds_at_least_one(wait: Duration) -> u64 {
    let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    secs.max(1)
}

/// Middleware: charges the request to its client and, past the limit, answers
/// `429` (enforce) or logs and serves it (shadow). [`HEALTH_PATH`] is exempt.
pub async fn rate_limit(
    State(limits): State<Arc<RateLimit>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if path == HEALTH_PATH {
        return next.run(request).await;
    }
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    let client = limits.charged_client(limits.identify(peer, request.headers()));
    let is_package = path == PACKAGE_PATH;
    if let Decision::Limited { retry_after_secs } = limits.check(&client, route_upfront_cost(path))
    {
        limits.log_limited(&client, path, retry_after_secs);
        if limits.enforce() {
            return AppError::RateLimited { retry_after_secs }.into_response();
        }
    }
    let response = next.run(request).await;
    if !is_package {
        return response;
    }
    // The package's build cost is settled once its status is known: a `304`
    // built nothing and owes nothing more. A client that cannot afford the
    // rest gets the `429` instead of the body it was already answered with
    // (the build was a cache hit or is now cached), so a stale `If-None-Match`
    // is never a discount on a full package.
    let settlement = route_settlement_cost(PACKAGE_PATH, response.status());
    if let Decision::Limited { retry_after_secs } = limits.check(&client, settlement) {
        limits.log_limited(&client, PACKAGE_PATH, retry_after_secs);
        if limits.enforce() {
            return AppError::RateLimited { retry_after_secs }.into_response();
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::connect_info::MockConnectInfo,
        http::{HeaderValue, StatusCode, header},
        middleware,
        routing::get,
    };
    use std::sync::Mutex;
    use tower::ServiceExt;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn settings(burst: u32) -> RateLimitSettings {
        RateLimitSettings {
            ip_tokens_per_second: 1,
            ip_burst: burst,
            key_tokens_per_second: 1,
            key_burst: burst * 2,
            ..RateLimitSettings::default()
        }
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn peer(addr: &str) -> Option<SocketAddr> {
        Some(SocketAddr::new(addr.parse().unwrap(), 40_000))
    }

    #[test]
    fn route_costs_weight_the_expensive_routes() {
        assert_eq!(route_cost("/meets"), DEFAULT_ROUTE_COST);
        assert_eq!(route_cost("/nope"), DEFAULT_ROUTE_COST);
        assert_eq!(route_cost("/meets/package"), PACKAGE_ROUTE_COST);
        assert_eq!(route_cost("/search"), SEARCH_ROUTE_COST);
        assert_eq!(
            route_cost("/lifting-results/by-names"),
            NAME_LIST_ROUTE_COST
        );
        assert_eq!(route_cost("/lifting-results/recent"), NAME_LIST_ROUTE_COST);
        assert_eq!(route_cost("/lifting-results/bests"), NAME_LIST_ROUTE_COST);
        assert_eq!(route_cost("/clubs/meet-stats"), MEET_STATS_ROUTE_COST);
    }

    #[test]
    fn a_package_revalidation_costs_one_token_and_a_body_the_full_route_cost() {
        assert_eq!(route_upfront_cost(PACKAGE_PATH), PACKAGE_REVALIDATE_COST);
        assert_eq!(PACKAGE_REVALIDATE_COST, 1);
        assert_eq!(
            route_settlement_cost(PACKAGE_PATH, StatusCode::NOT_MODIFIED),
            0
        );
        for status in [
            StatusCode::OK,
            StatusCode::NOT_FOUND,
            StatusCode::BAD_REQUEST,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert_eq!(
                route_upfront_cost(PACKAGE_PATH) + route_settlement_cost(PACKAGE_PATH, status),
                PACKAGE_ROUTE_COST,
                "{status}"
            );
        }
        // Every other route is paid in full before the handler runs.
        for path in ["/meets", "/search", "/lifting-results/by-names", "/nope"] {
            assert_eq!(route_upfront_cost(path), route_cost(path), "{path}");
            assert_eq!(route_settlement_cost(path, StatusCode::OK), 0, "{path}");
            assert_eq!(
                route_settlement_cost(path, StatusCode::NOT_MODIFIED),
                0,
                "{path}"
            );
        }
    }

    #[test]
    fn the_venue_history_refresh_model_matches_the_docs() {
        // 4 + ceil(1500 / 40) x 5 = 4 + 38 x 5.
        assert_eq!(HISTORY_REFRESH_COST, 194);
        // First hour of a meet day: 500 phones x 194 = 97,000 against
        // 1,200 + 40 x 3,600 = 145,200 (the compile-time assertion).
        assert_eq!(VENUE_DEVICES * HISTORY_REFRESH_COST, 97_000);
        assert_eq!(
            DEFAULT_IP_BURST + DEFAULT_IP_TOKENS_PER_SECOND * VENUE_FIRST_HOUR_SECS,
            145_200
        );
        // First minute: 200 phones x 194 = 38,800 against 1,200 + 2,400.
        assert_eq!(
            venue_first_minute_shortfall(VENUE_FIRST_MINUTE_DEVICES),
            38_800 - 3_600
        );
        assert_eq!(venue_first_minute_shortfall(0), 0);
        // The keyed burst is what docs/rate-limits.md says to raise toward;
        // even that covers only ~30 phones' first-open refreshes, so the
        // recommendation is the burst plus watching the shadow logs.
        assert_eq!(DEFAULT_KEY_BURST / HISTORY_REFRESH_COST, 30);
    }

    /// A package handler that answers `304` to any `If-None-Match`, else `200`.
    fn package_router(limits: Arc<RateLimit>) -> Router {
        async fn package(headers: HeaderMap) -> Response {
            if headers.contains_key(header::IF_NONE_MATCH) {
                StatusCode::NOT_MODIFIED.into_response()
            } else {
                "package".into_response()
            }
        }
        Router::new()
            .route(PACKAGE_PATH, get(package))
            .layer(middleware::from_fn_with_state(limits, rate_limit))
            .layer(MockConnectInfo(SocketAddr::from((
                [203, 0, 113, 9],
                40_000,
            ))))
    }

    async fn package_status(app: &Router, revalidate: bool) -> StatusCode {
        let mut request = Request::builder().uri("/meets/package?meet=Test");
        if revalidate {
            request = request.header(header::IF_NONE_MATCH, "\"etag\"");
        }
        app.clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn package_revalidations_spend_one_token_and_bodies_settle_the_rest() {
        // Capture this test's own warnings. Every test that reaches
        // `log_limited` installs a scoped subscriber before its first request:
        // hitting that callsite with no subscriber at all races the interest
        // cache the shadow-mode test relies on.
        let logs = CapturedLogs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        // Bucket of PACKAGE_ROUTE_COST + 1 tokens, refilled 1/s, enforcing.
        let burst = PACKAGE_ROUTE_COST + 1;
        let limits = || {
            let enforcing = RateLimitSettings {
                enforce: true,
                ..settings(burst)
            };
            Arc::new(RateLimit::new(&enforcing, ApiKeys::none()).unwrap())
        };

        // `burst` revalidations fit; a fourth would not if a 304 cost 4.
        let app = package_router(limits());
        for _ in 0..burst {
            assert_eq!(package_status(&app, true).await, StatusCode::NOT_MODIFIED);
        }
        assert_eq!(
            package_status(&app, true).await,
            StatusCode::TOO_MANY_REQUESTS
        );

        // A build (4) then a revalidation (1) fit exactly; the next 304 is over.
        let app = package_router(limits());
        assert_eq!(package_status(&app, false).await, StatusCode::OK);
        assert_eq!(package_status(&app, true).await, StatusCode::NOT_MODIFIED);
        assert_eq!(
            package_status(&app, true).await,
            StatusCode::TOO_MANY_REQUESTS
        );

        // Two builds need 8: the second passes the up-front token but cannot
        // settle the rest, so it is answered 429, not with a discounted body.
        let app = package_router(limits());
        assert_eq!(package_status(&app, false).await, StatusCode::OK);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/meets/package?meet=Test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().get(header::RETRY_AFTER).is_some());

        let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert!(logged.contains("path=/meets/package"), "{logged}");
        assert!(logged.contains("enforced=true"), "{logged}");
        assert!(
            !logged.contains("meet=Test"),
            "the query is never logged: {logged}"
        );
    }

    #[test]
    fn parses_api_keys_and_rejects_bad_ones() {
        let keys = ApiKeys::parse(&format!(" atlas:{SECRET} , cli-beta:{SECRET}x ,")).unwrap();
        assert_eq!(keys.names(), ["atlas", "cli-beta"]);
        assert_eq!(keys.find(SECRET.as_bytes()), Some(0));
        assert_eq!(keys.find(format!("{SECRET}x").as_bytes()), Some(1));
        assert_eq!(keys.find(b"0123456789abcdef0123456789abcdeX"), None);
        assert_eq!(keys.find(b"short"), None);
        assert!(ApiKeys::parse("").unwrap().is_empty());

        for bad in [
            "atlas".to_string(),
            "atlas:short".to_string(),
            format!("Atlas:{SECRET}"),
            format!(":{SECRET}"),
            format!("atlas:{SECRET}!"),
            format!("atlas:{SECRET},atlas:{SECRET}x"),
            format!("atlas:{SECRET},other:{SECRET}"),
        ] {
            let error = ApiKeys::parse(&bad).expect_err(&bad).to_string();
            assert!(
                !error.contains(SECRET),
                "errors never echo a secret: {error}"
            );
        }
        let too_many: Vec<String> = (0..=MAX_API_KEYS)
            .map(|index| format!("k{index}:{SECRET}{index:04}"))
            .collect();
        assert!(ApiKeys::parse(&too_many.join(",")).is_err());
    }

    #[test]
    fn debug_output_names_keys_without_digests() {
        let keys = ApiKeys::parse(&format!("atlas:{SECRET}")).unwrap();
        assert_eq!(format!("{keys:?}"), r#"["atlas"]"#);
    }

    #[test]
    fn cidrs_parse_and_match() {
        let loopback = IpNet::parse("127.0.0.0/8").unwrap();
        assert!(loopback.contains("127.0.0.1".parse().unwrap()));
        assert!(loopback.contains("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!loopback.contains("10.0.0.1".parse().unwrap()));
        assert!(!loopback.contains("::1".parse().unwrap()));
        let host = IpNet::parse("172.18.0.1").unwrap();
        assert!(host.contains("172.18.0.1".parse().unwrap()));
        assert!(!host.contains("172.18.0.2".parse().unwrap()));
        assert!(
            IpNet::parse("::1/128")
                .unwrap()
                .contains("::1".parse().unwrap())
        );
        assert_eq!(IpNet::parse("0.0.0.0/0").unwrap().to_string(), "0.0.0.0/0");
        for bad in ["", "10.0.0.0/33", "::/129", "localhost", "10.0.0.0/x"] {
            assert!(IpNet::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn ipv6_clients_group_by_64_and_mapped_ipv4_is_ipv4() {
        assert_eq!(
            client_network("2001:db8:1:2:aaaa::1".parse().unwrap()),
            "2001:db8:1:2::".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            client_network("::ffff:203.0.113.7".parse().unwrap()),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            client_network("203.0.113.7".parse().unwrap()),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn the_rightmost_forwarded_entry_of_the_last_header_wins() {
        let map = headers(&[
            ("x-forwarded-for", "198.51.100.1, 198.51.100.2"),
            ("x-forwarded-for", "10.9.9.9 , 203.0.113.7"),
        ]);
        assert_eq!(
            rightmost_forwarded_for(&map),
            Some("203.0.113.7".parse().unwrap())
        );
        let garbage = headers(&[("x-forwarded-for", "203.0.113.7, not-an-ip")]);
        assert_eq!(rightmost_forwarded_for(&garbage), None);
        assert_eq!(rightmost_forwarded_for(&HeaderMap::new()), None);
    }

    #[test]
    fn identity_trusts_forwarded_for_only_from_a_trusted_peer() {
        let limits = RateLimit::new(&settings(10), ApiKeys::none()).unwrap();
        let forwarded = headers(&[("x-forwarded-for", "198.51.100.1, 203.0.113.7")]);
        let client = "203.0.113.7".parse().unwrap();
        assert_eq!(
            limits.identify(peer("127.0.0.1"), &forwarded),
            Client::Ip(client)
        );
        assert_eq!(
            limits.identify(peer("::ffff:127.0.0.1"), &forwarded),
            Client::Ip(client)
        );
        assert_eq!(
            limits.identify(peer("192.0.2.9"), &forwarded),
            Client::Ip("192.0.2.9".parse().unwrap())
        );
        // An unparsable entry falls back to the (trusted) peer, not to a
        // client-chosen address.
        let garbage = headers(&[("x-forwarded-for", "junk")]);
        assert_eq!(
            limits.identify(peer("127.0.0.1"), &garbage),
            Client::Ip("127.0.0.1".parse().unwrap())
        );
        assert_eq!(
            limits.identify(None, &forwarded),
            Client::Ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
        );

        let off = RateLimit::new(
            &RateLimitSettings {
                trust_forwarded_for: false,
                ..settings(10)
            },
            ApiKeys::none(),
        )
        .unwrap();
        assert_eq!(
            off.identify(peer("127.0.0.1"), &forwarded),
            Client::Ip("127.0.0.1".parse().unwrap())
        );
    }

    #[test]
    fn a_valid_key_is_its_own_client_and_a_bad_one_is_anonymous() {
        let keys = ApiKeys::parse(&format!("atlas:{SECRET}")).unwrap();
        let limits = RateLimit::new(&settings(10), keys).unwrap();
        assert_eq!(
            limits.identify(peer("127.0.0.1"), &headers(&[(API_KEY_HEADER, SECRET)])),
            Client::Key(0)
        );
        assert_eq!(
            limits.identify(peer("127.0.0.1"), &headers(&[(API_KEY_HEADER, "nope")])),
            Client::Ip("127.0.0.1".parse().unwrap())
        );
    }

    #[test]
    fn settings_that_could_never_serve_a_route_are_rejected() {
        assert!(RateLimit::new(&settings(MAX_ROUTE_COST - 1), ApiKeys::none()).is_err());
        let zero_rate = RateLimitSettings {
            ip_tokens_per_second: 0,
            ..RateLimitSettings::default()
        };
        assert!(RateLimit::new(&zero_rate, ApiKeys::none()).is_err());
        let bad_proxy = RateLimitSettings {
            trusted_proxies: "127.0.0.1,nonsense".to_string(),
            ..RateLimitSettings::default()
        };
        assert!(RateLimit::new(&bad_proxy, ApiKeys::none()).is_err());
        assert!(RateLimit::new(&RateLimitSettings::default(), ApiKeys::none()).is_ok());
    }

    #[test]
    fn a_full_address_map_folds_new_addresses_into_bounded_overflow_slots() {
        let cap = 3;
        let limits = RateLimit::new(&RateLimitSettings::default(), ApiKeys::none())
            .unwrap()
            .with_max_tracked(cap);
        for last in 1..=cap {
            let client =
                limits.charged_client(Client::Ip(format!("203.0.113.{last}").parse().unwrap()));
            assert!(
                matches!(client, Client::Ip(IpAddr::V4(_))),
                "below the cap: own bucket"
            );
            assert_eq!(limits.check(&client, 1), Decision::Allow);
        }
        assert_eq!(limits.tracked_clients(), cap);

        // A spray of far more distinct addresses than the cap plus slots.
        let spray = 4 * usize::from(OVERFLOW_BUCKETS);
        for n in 0..spray {
            let addr = IpAddr::V4(std::net::Ipv4Addr::from(0x0a00_0000 + n as u32));
            let client = limits.charged_client(Client::Ip(addr));
            match &client {
                Client::Ip(IpAddr::V6(v6)) => {
                    assert_eq!(v6.segments()[0], OVERFLOW_PREFIX);
                    assert!(v6.segments()[7] < OVERFLOW_BUCKETS);
                }
                other => panic!("past the cap an address must fold, got {other:?}"),
            }
            assert_eq!(limits.kind(&client), "ip-overflow");
            limits.check(&client, 1);
        }
        let tracked = limits.tracked_clients();
        assert!(
            tracked <= cap + usize::from(OVERFLOW_BUCKETS),
            "map grew to {tracked}"
        );

        // Keys are never folded.
        assert_eq!(limits.charged_client(Client::Key(0)), Client::Key(0));
    }

    #[test]
    fn retry_after_rounds_up_to_whole_seconds() {
        assert_eq!(whole_seconds_at_least_one(Duration::ZERO), 1);
        assert_eq!(whole_seconds_at_least_one(Duration::from_millis(1)), 1);
        assert_eq!(whole_seconds_at_least_one(Duration::from_millis(1_001)), 2);
        assert_eq!(whole_seconds_at_least_one(Duration::from_secs(4)), 4);
    }

    #[test]
    fn a_bucket_limits_past_its_burst_and_reports_the_refill_time() {
        let limits = RateLimit::new(&settings(SEARCH_ROUTE_COST), ApiKeys::none()).unwrap();
        let client = Client::Ip("203.0.113.7".parse().unwrap());
        assert_eq!(limits.check(&client, SEARCH_ROUTE_COST), Decision::Allow);
        match limits.check(&client, SEARCH_ROUTE_COST) {
            Decision::Limited { retry_after_secs } => {
                assert!((4..=5).contains(&retry_after_secs), "{retry_after_secs}")
            }
            Decision::Allow => panic!("a spent bucket must limit"),
        }
        // Another client is unaffected.
        let other = Client::Ip("203.0.113.8".parse().unwrap());
        assert_eq!(limits.check(&other, 1), Decision::Allow);
    }

    #[test]
    fn eviction_drops_refilled_clients() {
        let fast = RateLimitSettings {
            ip_tokens_per_second: 20,
            ..settings(MAX_ROUTE_COST)
        };
        let limits = RateLimit::new(&fast, ApiKeys::none()).unwrap();
        limits.check(&Client::Ip("203.0.113.7".parse().unwrap()), 1);
        limits.evict_idle();
        assert_eq!(limits.tracked_clients(), 1, "a bucket in use is kept");
        // At 20 tokens/s the spent token is back after 50ms; governor keeps a
        // key one more token interval before it counts as fresh.
        std::thread::sleep(Duration::from_millis(250));
        limits.evict_idle();
        assert_eq!(limits.tracked_clients(), 0);
    }

    /// Collects formatted log lines so a test can read what was logged.
    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn shadow_mode_serves_everything_and_logs_one_redacted_line_per_client() {
        let logs = CapturedLogs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .finish();
        // The current-thread test runtime polls the whole request on this
        // thread, so the thread-local default catches the middleware's event.
        let _guard = tracing::subscriber::set_default(subscriber);

        let limits = Arc::new(RateLimit::new(&settings(MAX_ROUTE_COST), ApiKeys::none()).unwrap());
        let app = Router::new()
            .route("/search", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(limits, rate_limit))
            .layer(MockConnectInfo(SocketAddr::from((
                [203, 0, 113, 7],
                40_000,
            ))));

        for _ in 0..5 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/search?query=Secret%20Athlete")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert!(response.headers().get(header::RETRY_AFTER).is_none());
            to_bytes(response.into_body(), 64).await.unwrap();
        }

        let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert_eq!(logged.lines().count(), 1, "{logged}");
        assert!(logged.contains("rate limit exceeded"), "{logged}");
        assert!(logged.contains("bucket=ip"), "{logged}");
        assert!(logged.contains("path=/search"), "{logged}");
        assert!(logged.contains("enforced=false"), "{logged}");
        assert!(logged.contains("client_tag="), "{logged}");
        assert!(
            !logged.contains("Secret"),
            "the query is never logged: {logged}"
        );
        assert!(
            !logged.contains("203.0.113.7"),
            "the address is never logged: {logged}"
        );
    }
}
