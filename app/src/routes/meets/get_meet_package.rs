use crate::{
    AppError, AppState,
    common::{
        client::ClientVersion,
        http_cache::{json_response, strong_etag},
        names::{normalize_name, normalized_name_sql},
    },
    routes::results::types::{LiftingResults, best_lifts_columns, lifting_result_columns},
};
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetPackageParams {
    pub meet: String,
    pub history_cutoff_date: Option<String>,
    /// Comma-separated subset of `year_bests`, `recent_results`,
    /// `attempt_estimates`. Absent means all three, the historical shape.
    pub include: Option<String>,
}

/// Which optional sections a package carries. Parsed from `include=`; the
/// set is part of the cache key, and because omitted sections are left out of
/// the body, it is part of the `ETag` as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageInclude {
    pub year_bests: bool,
    pub recent_results: bool,
    pub attempt_estimates: bool,
}

impl PackageInclude {
    pub const ALL: Self = Self {
        year_bests: true,
        recent_results: true,
        attempt_estimates: true,
    };

    /// `None` or blank is [`Self::ALL`]; an unknown section name is a `400`
    /// for every client, since no shipped app sends `include` at all.
    pub fn parse(raw: Option<&str>) -> Result<Self, AppError> {
        let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
            return Ok(Self::ALL);
        };
        let mut include = Self {
            year_bests: false,
            recent_results: false,
            attempt_estimates: false,
        };
        for section in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match section {
                "year_bests" => include.year_bests = true,
                "recent_results" => include.recent_results = true,
                "attempt_estimates" => include.attempt_estimates = true,
                other => {
                    return Err(AppError::Validation(format!(
                        "include has unknown section '{other}'; expected year_bests, recent_results, attempt_estimates"
                    )));
                }
            }
        }
        Ok(include)
    }

    /// Both `recent_results` and `attempt_estimates` are built from the same
    /// history rows; `year_bests` has its own aggregate and needs neither.
    fn needs_history(&self) -> bool {
        self.recent_results || self.attempt_estimates
    }

    fn cache_key_part(&self) -> String {
        format!(
            "{}{}{}",
            u8::from(self.year_bests),
            u8::from(self.recent_results),
            u8::from(self.attempt_estimates)
        )
    }
}

/// How long a built package is served from cache before being rebuilt, set via
/// APP_PACKAGE_CACHE_TTL_SECS (default 3600s / 1 hour).
///
/// The TTL is a backstop. Invalidation after ingest is immediate: every request
/// reads a cheap freshness stamp for the meet ([`FRESHNESS_SQL`]) and a cached
/// body is only served while its stamp still matches.
const DEFAULT_PACKAGE_CACHE_TTL_SECS: u64 = 60 * 60;
static PACKAGE_CACHE_TTL: LazyLock<Duration> = LazyLock::new(|| {
    let secs = std::env::var("APP_PACKAGE_CACHE_TTL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_PACKAGE_CACHE_TTL_SECS);
    Duration::from_secs(secs)
});

/// One `lifting_results` row, the same shape every result endpoint returns.
pub type PackageLiftingResult = LiftingResults;

const MEET_RESULTS_SQL: &str = concat!(
    r#"
        SELECT
            "#,
    lifting_result_columns!(),
    r#"
        FROM lifting_results
        WHERE meet = $1
        ORDER BY name, date DESC
        "#
);

const ATHLETE_HISTORY_SQL: &str = concat!(
    r#"
        SELECT
            "#,
    lifting_result_columns!(),
    r#"
        FROM lifting_results
        WHERE "#,
    normalized_name_sql!(),
    r#" = ANY($1::text[])
            AND date >= $2
        ORDER BY name, date DESC
        "#
);

/// Bests over the year that starts one year after the caller's
/// `history_cutoff_date` (`$2`). The app sends a two-year cutoff for recent
/// results, so this is "the last year" on the caller's clock, the same window
/// `/lifting-results/bests` gets from its `cutoff_date`, rather than the
/// server's `CURRENT_DATE`.
const YEAR_BESTS_BY_NAME_SQL: &str = concat!(
    r#"
        SELECT
            "#,
    normalized_name_sql!(),
    r#" AS name,
            "#,
    best_lifts_columns!(),
    r#"
        FROM lifting_results
        WHERE "#,
    normalized_name_sql!(),
    r#" = ANY($1::text[])
            AND date >= (($2::date + INTERVAL '1 year')::date)::text
        GROUP BY "#,
    normalized_name_sql!(),
    r#"
        "#
);

const MEET_SQL: &str = r#"
        SELECT
            convex_id AS id,
            name,
            federation,
            status,
            start_date::text AS start_date,
            end_date::text AS end_date,
            time_zone,
            venue_name,
            venue_street,
            venue_city,
            venue_state,
            venue_zip,
            venue_map_pdf_url,
            venue_map_apple_url
        FROM meets
        WHERE name = $1
        "#;

const SCHEDULE_SQL: &str = r#"
        SELECT date, session_id, start_time, weigh_in_time, platform, weight_class
        FROM session_schedule
        WHERE meet = $1
        ORDER BY date, session_id, platform
        "#;

const ATHLETES_SQL: &str = r#"
        SELECT
            a.member_id,
            a.name,
            a.age,
            a.club,
            a.wso,
            a.gender,
            a.weight_class,
            a.entry_total,
            a.adaptive,
            a.session_number,
            a.session_platform,
            s.date,
            s.start_time,
            s.weigh_in_time
        FROM athletes a
        LEFT JOIN session_schedule s
            ON s.meet = a.meet
            AND s.session_id = a.session_number
            AND s.platform = a.session_platform
        WHERE a.meet = $1
        ORDER BY a.name
        "#;

/// Freshness stamp for one meet, read on every package request before the
/// cache lookup.
///
/// None of `athletes`, `session_schedule`, or `lifting_results` carries an
/// `updated_at`, so the stamp is, per table for this meet, the row count plus
/// the newest `xmin` (the transaction id that last wrote each row), each from
/// one index scan on `meet`. A replace ingest (delete + insert), an upsert
/// that rewrites a row in place, and a deletion all change one of those. The
/// meet row's own `xmin` catches any write to it, including the Slack
/// venue-map update, which does not touch `updated_at`.
///
/// Scope: the stamp covers this meet's rows only. `recent_results_by_name`,
/// `year_bests_by_name` and `attempt_estimates` are built from the lifters'
/// results at *other* meets, so a new result elsewhere reaches the package
/// when the TTL expires, not immediately. `xmin` is also not monotonic across
/// transaction-id wraparound; the TTL is the backstop for both. A missing
/// meet is a `404` here, before anything is built.
const FRESHNESS_SQL: &str = r#"
        SELECT
            m.updated_at AS meet_updated_at,
            m.xmin::text::bigint AS meet_tx,
            a.n AS athlete_rows,
            a.tx AS athlete_tx,
            s.n AS schedule_rows,
            s.tx AS schedule_tx,
            r.n AS result_rows,
            r.tx AS result_tx
        FROM meets m
        CROSS JOIN LATERAL (
            SELECT COUNT(*) AS n, COALESCE(MAX(xmin::text::bigint), 0) AS tx
            FROM athletes WHERE meet = m.name
        ) a
        CROSS JOIN LATERAL (
            SELECT COUNT(*) AS n, COALESCE(MAX(xmin::text::bigint), 0) AS tx
            FROM session_schedule WHERE meet = m.name
        ) s
        CROSS JOIN LATERAL (
            SELECT COUNT(*) AS n, COALESCE(MAX(xmin::text::bigint), 0) AS tx
            FROM lifting_results WHERE meet = m.name
        ) r
        WHERE m.name = $1
        LIMIT 1
        "#;

#[derive(Debug, FromRow)]
struct FreshnessRow {
    meet_updated_at: i64,
    meet_tx: i64,
    athlete_rows: i64,
    athlete_tx: i64,
    schedule_rows: i64,
    schedule_tx: i64,
    result_rows: i64,
    result_tx: i64,
}

async fn freshness_stamp(state: &AppState, meet: &str) -> Result<String, AppError> {
    let row = sqlx::query_as::<_, FreshnessRow>(FRESHNESS_SQL)
        .bind(meet)
        .fetch_one(&state.db)
        .await?;
    Ok(format!(
        "{}:{}:{}:{}:{}:{}:{}:{}",
        row.meet_updated_at,
        row.meet_tx,
        row.athlete_rows,
        row.athlete_tx,
        row.schedule_rows,
        row.schedule_tx,
        row.result_rows,
        row.result_tx
    ))
}

struct CachedPackage {
    body: Bytes,
    etag: HeaderValue,
    stamp: String,
    inserted: Instant,
}

/// Process-wide cache of pre-serialized package bodies, keyed by
/// meet + cutoff + include set. The freshness stamp lives in the entry, so a
/// meet has one entry per key and an ingest replaces it rather than leaving a
/// stale twin behind.
static PACKAGE_CACHE: LazyLock<RwLock<HashMap<String, CachedPackage>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
const MAX_PACKAGE_CACHE_ENTRIES: usize = 32;
/// Bytes of one cached package. A national championship (about 1,000 lifters
/// with two years of history and attempt estimates) serializes to roughly
/// 6-8 MiB, so the previous 8 MiB cap let exactly the packages that are most
/// expensive to rebuild fall out of the cache. 16 MiB keeps them with headroom.
const MAX_CACHED_PACKAGE_BYTES: usize = 16 * 1024 * 1024;
/// Total bytes across all cached packages: six national-size packages, or
/// dozens of local meets. The API container has no memory cap, so this is
/// the bound on the cache's footprint.
const MAX_PACKAGE_CACHE_BYTES: usize = 96 * 1024 * 1024;

fn cached_package(key: &str, stamp: &str) -> Option<(Bytes, HeaderValue)> {
    let cache = PACKAGE_CACHE.read().ok()?;
    let entry = cache.get(key)?;
    (entry.stamp == stamp && entry.inserted.elapsed() < *PACKAGE_CACHE_TTL)
        .then(|| (entry.body.clone(), entry.etag.clone()))
}

fn store_package(key: &str, stamp: String, body: Bytes, etag: HeaderValue) {
    if body.len() > MAX_CACHED_PACKAGE_BYTES {
        return;
    }
    if let Ok(mut cache) = PACKAGE_CACHE.write() {
        cache.retain(|_, entry| entry.inserted.elapsed() < *PACKAGE_CACHE_TTL);
        cache.remove(key);
        while cache.len() >= MAX_PACKAGE_CACHE_ENTRIES
            || cache.values().map(|entry| entry.body.len()).sum::<usize>() + body.len()
                > MAX_PACKAGE_CACHE_BYTES
        {
            let Some(oldest_key) = cache
                .iter()
                .max_by_key(|(_, entry)| entry.inserted.elapsed())
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            cache.remove(&oldest_key);
        }
        cache.insert(
            key.to_string(),
            CachedPackage {
                body,
                etag,
                stamp,
                inserted: Instant::now(),
            },
        );
    }
}

/// Single-flight guard: one build per cache key at a time.
///
/// When a cached package expires (or an ingest changes its stamp) during a
/// meet weekend, every phone in the venue misses at once. Without this, each
/// miss ran its own rebuild -- N times the queries and N times the
/// serialization for one identical body. Now the first miss builds while the
/// rest wait on its per-key lock and then read the entry it stored.
///
/// The map holds one entry per key currently being built and the last holder
/// removes it, so it is bounded by in-flight distinct keys, not by history.
static IN_FLIGHT: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Package builds that may run at once, across every cache key. The route is
/// public and each `history_cutoff_date` is its own key, so the per-key lock
/// alone does not bound the work; a build also outlives its request (it runs
/// in its own task), so without this cap a burst of distinct keys would pile
/// up detached builds on the database pool.
const MAX_CONCURRENT_PACKAGE_BUILDS: usize = 4;
/// How long a request waits for a build slot before answering `503`. Under
/// the 15s request ceiling, so the wait is cancelled with the request and
/// never leaves work behind.
const PACKAGE_BUILD_SLOT_WAIT: Duration = Duration::from_secs(10);
static BUILD_SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_PACKAGE_BUILDS)));

struct BuildSlot {
    key: String,
    lock: Arc<tokio::sync::Mutex<()>>,
}

fn build_slot(key: &str) -> BuildSlot {
    let mut in_flight = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    let lock = in_flight.entry(key.to_string()).or_default().clone();
    BuildSlot {
        key: key.to_string(),
        lock,
    }
}

impl Drop for BuildSlot {
    fn drop(&mut self) {
        let mut in_flight = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        // The map's reference plus ours: nobody else is waiting on this key.
        // Checked under the map lock, so no new waiter can clone in between.
        if in_flight
            .get(&self.key)
            .is_some_and(|slot| Arc::ptr_eq(slot, &self.lock))
            && Arc::strong_count(&self.lock) <= 2
        {
            in_flight.remove(&self.key);
        }
    }
}

/// How many times each meet's package has been built since process start.
/// A diagnostic for the single-flight guard and the freshness stamp: bounded
/// by clearing once it outgrows twice the cache, since only the recent counts
/// are ever read.
static BUILD_COUNTS: LazyLock<Mutex<HashMap<String, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn record_build(meet: &str) {
    let mut counts = BUILD_COUNTS.lock().unwrap_or_else(|e| e.into_inner());
    if counts.len() >= MAX_PACKAGE_CACHE_ENTRIES * 2 && !counts.contains_key(meet) {
        counts.clear();
    }
    *counts.entry(meet.to_string()).or_default() += 1;
}

/// Package builds for `meet` since process start (see [`BUILD_COUNTS`]).
pub fn package_builds(meet: &str) -> u64 {
    BUILD_COUNTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(meet)
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    /// `PACKAGE_CACHE` is process-wide, so the tests that clear and fill it
    /// cannot run concurrently: one clearing the cache mid-fill makes the
    /// other's count assertion fail. Serialize them.
    static CACHE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn package_cache_never_exceeds_entry_limit() {
        let _serialized = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        PACKAGE_CACHE.write().unwrap().clear();
        for index in 0..(MAX_PACKAGE_CACHE_ENTRIES + 5) {
            store_package(
                &format!("meet-{index}"),
                "stamp".to_string(),
                Bytes::from_static(b"{}"),
                strong_etag(b"{}"),
            );
        }
        assert_eq!(
            PACKAGE_CACHE.read().unwrap().len(),
            MAX_PACKAGE_CACHE_ENTRIES
        );
        PACKAGE_CACHE.write().unwrap().clear();
    }

    #[test]
    fn oversized_packages_are_not_cached() {
        let _serialized = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        PACKAGE_CACHE.write().unwrap().clear();
        store_package(
            "oversized",
            "stamp".to_string(),
            Bytes::from(vec![0; MAX_CACHED_PACKAGE_BYTES + 1]),
            strong_etag(b"oversized"),
        );
        assert!(!PACKAGE_CACHE.read().unwrap().contains_key("oversized"));
    }

    #[test]
    fn a_changed_freshness_stamp_is_a_cache_miss() {
        let _serialized = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        PACKAGE_CACHE.write().unwrap().clear();
        store_package(
            "stamped",
            "1:2:3".to_string(),
            Bytes::from_static(b"{}"),
            strong_etag(b"{}"),
        );
        assert!(cached_package("stamped", "1:2:3").is_some());
        assert!(cached_package("stamped", "1:2:4").is_none());
        PACKAGE_CACHE.write().unwrap().clear();
    }

    #[test]
    fn include_parses_the_three_sections_and_rejects_others() {
        assert_eq!(PackageInclude::parse(None).unwrap(), PackageInclude::ALL);
        assert_eq!(
            PackageInclude::parse(Some("  ")).unwrap(),
            PackageInclude::ALL
        );
        assert_eq!(
            PackageInclude::parse(Some("year_bests")).unwrap(),
            PackageInclude {
                year_bests: true,
                recent_results: false,
                attempt_estimates: false,
            }
        );
        assert_eq!(
            PackageInclude::parse(Some("attempt_estimates, recent_results")).unwrap(),
            PackageInclude {
                year_bests: false,
                recent_results: true,
                attempt_estimates: true,
            }
        );
        assert!(PackageInclude::parse(Some("schedule")).is_err());
        assert_ne!(
            PackageInclude::ALL.cache_key_part(),
            PackageInclude::parse(Some("year_bests"))
                .unwrap()
                .cache_key_part()
        );
    }

    #[test]
    fn build_slots_are_shared_per_key_and_released_by_the_last_holder() {
        let first = build_slot("slot-key");
        let second = build_slot("slot-key");
        assert!(Arc::ptr_eq(&first.lock, &second.lock));
        drop(first);
        assert!(IN_FLIGHT.lock().unwrap().contains_key("slot-key"));
        drop(second);
        assert!(!IN_FLIGHT.lock().unwrap().contains_key("slot-key"));
    }
}

#[derive(Debug, Serialize)]
pub struct MeetPackage {
    pub meet: PackageMeet,
    pub schedule: Vec<PackageScheduleDay>,
    pub athletes: Vec<PackageAthlete>,
    pub meet_results: Vec<PackageLiftingResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_estimates: Option<Vec<PackageAttemptEstimateSession>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year_bests_by_name: Option<BTreeMap<String, YearBests>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_results_by_name: Option<BTreeMap<String, Vec<PackageLiftingResult>>>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct PackageMeet {
    pub id: String,
    pub name: String,
    pub federation: String,
    pub status: String,
    pub start_date: String,
    pub end_date: String,
    pub time_zone: String,
    pub venue_name: String,
    pub venue_street: String,
    pub venue_city: String,
    pub venue_state: String,
    pub venue_zip: String,
    pub venue_map_pdf_url: Option<String>,
    pub venue_map_apple_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PackageScheduleDay {
    pub date: String,
    pub sessions: Vec<PackageScheduleSession>,
}

#[derive(Debug, Serialize)]
pub struct PackageScheduleSession {
    pub session_id: f64,
    pub start_time: String,
    pub weigh_in_time: String,
    pub platforms: Vec<PackageSchedulePlatform>,
}

#[derive(Debug, Serialize)]
pub struct PackageSchedulePlatform {
    pub platform: String,
    pub weight_class: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageAthlete {
    pub member_id: String,
    pub name: String,
    pub age: f64,
    pub club: String,
    pub wso: Option<String>,
    pub gender: String,
    pub weight_class: String,
    pub entry_total: f64,
    pub adaptive: bool,
    pub session: Option<PackageAthleteSession>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageAthleteSession {
    pub session_number: f64,
    pub session_platform: String,
    pub date: Option<String>,
    pub start_time: Option<String>,
    pub weigh_in_time: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct YearBests {
    pub best_snatch: f64,
    pub best_cj: f64,
    pub best_total: f64,
}

#[derive(Debug, FromRow)]
struct YearBestsByNameRow {
    name: String,
    best_snatch: f64,
    best_cj: f64,
    best_total: f64,
}

#[derive(Debug, Serialize)]
pub struct PackageAttemptEstimateSession {
    pub session_number: f64,
    pub platform: String,
    pub date: Option<String>,
    pub start_time: Option<String>,
    pub weigh_in_time: Option<String>,
    pub estimates: Vec<PackageAttemptEstimate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageAttemptEstimate {
    pub athlete_id: String,
    pub athlete_name: String,
    pub weight_class: String,
    pub entry_total: f64,
    pub history_result_count: usize,
    pub snatch: PackageLiftAttemptEstimate,
    pub clean_and_jerk: PackageLiftAttemptEstimate,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageLiftAttemptEstimate {
    pub attempts: Vec<f64>,
    pub attempts_out: usize,
    pub average_increase: AttemptIncrease,
    pub make_rate: f64,
    pub historical_best: Option<f64>,
    pub source: AttemptEstimateSource,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct AttemptIncrease {
    pub first_to_second: f64,
    pub second_to_third: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptEstimateSource {
    History,
    EntryTotal,
    Unavailable,
}

#[derive(Debug, Clone, Copy)]
enum LiftType {
    Snatch,
    CleanAndJerk,
}

#[derive(Debug)]
struct AttemptData {
    athlete_id: String,
    weight: f64,
    attempt_number: usize,
}

#[derive(Debug)]
struct TempAttemptEstimate<'a> {
    athlete: PackageAthlete,
    /// Borrowed slice out of [`HistoryByName`]; the rows themselves are owned
    /// by the caller's `history_rows` and are never cloned per athlete.
    history: &'a [&'a PackageLiftingResult],
    best_snatch: Option<f64>,
    best_cj: Option<f64>,
    avg_snatch_increase: AttemptIncrease,
    avg_cj_increase: AttemptIncrease,
    snatch_make_rate: f64,
    cj_make_rate: f64,
}

#[derive(Debug, FromRow)]
struct ScheduleRow {
    date: String,
    session_id: f64,
    start_time: String,
    weigh_in_time: String,
    platform: String,
    weight_class: String,
}

#[derive(Debug, FromRow)]
struct AthletePackageRow {
    member_id: String,
    name: String,
    age: f64,
    club: String,
    wso: Option<String>,
    gender: String,
    weight_class: String,
    entry_total: f64,
    adaptive: bool,
    session_number: Option<f64>,
    session_platform: Option<String>,
    date: Option<String>,
    start_time: Option<String>,
    weigh_in_time: Option<String>,
}

/// /meets/package endpoint
///
/// curl 'https://api.meetcal.app/meets/package?meet=2026%20Ohio%20WSO%20Championships&history_cutoff_date=2024-06-13' | jq .
///
/// This endpoint returns a selected meet package for app screens that share the same meet data:
/// schedule, start list, schedule details, offline download, attempt estimator, and cached bests.
/// If history_cutoff_date is omitted, recent_results_by_name and year_bests_by_name are empty.
/// year_bests_by_name covers the year starting one year after history_cutoff_date (the app sends
/// a two-year cutoff, so that is the last year on the app's clock).
///
/// Optional `include=year_bests,recent_results,attempt_estimates` (any subset) leaves the other
/// sections out of the body entirely; absent means all three. A blank `meet` is `400` for a
/// 6.2.0+ client and `404` for a legacy one; an unknown meet is `404`.
///
/// The body carries a strong `ETag` and answers a matching `If-None-Match` with `304`. Bodies are
/// cached per meet + cutoff + include set, revalidated against a per-meet freshness stamp on every
/// request (a write to this meet's rows invalidates immediately; history from other meets waits
/// for the one-hour TTL), and built at most once at a time per key.
///
/// {
///   "meet": {
///     "id": "meet_ohio_2026",
///     "name": "2026 Ohio WSO Championships",
///     "federation": "USAW",
///     "status": "completed",
///     "start_date": "2026-05-01",
///     "end_date": "2026-05-03",
///     "time_zone": "America/New_York",
///     "venue_name": "Ohio Expo Center",
///     "venue_street": "717 E 17th Ave",
///     "venue_city": "Columbus",
///     "venue_state": "OH",
///     "venue_zip": "43211"
///   },
///   "schedule": [],
///   "athletes": [],
///   "meet_results": [],
///   "attempt_estimates": [],
///   "year_bests_by_name": {},
///   "recent_results_by_name": {}
/// }
pub async fn get_meet_package(
    State(state): State<AppState>,
    client: ClientVersion,
    headers: HeaderMap,
    Query(params): Query<MeetPackageParams>,
) -> Result<Response, AppError> {
    let if_none_match = headers.get(header::IF_NONE_MATCH);
    client.require_non_empty("meet", &params.meet)?;
    crate::common::query::require_iso_date(
        "history_cutoff_date",
        params.history_cutoff_date.as_deref(),
    )?;
    let include = PackageInclude::parse(params.include.as_deref())?;

    let stamp = freshness_stamp(&state, &params.meet).await?;
    let cache_key = format!(
        "{}|{}|{}",
        params.meet,
        params.history_cutoff_date.as_deref().unwrap_or(""),
        include.cache_key_part()
    );
    if let Some((body, etag)) = cached_package(&cache_key, &stamp) {
        return Ok(json_response(body, etag, None, if_none_match));
    }

    let slot = build_slot(&cache_key);
    let building = slot.lock.clone().lock_owned().await;
    // Another request may have built this key while we waited for the slot.
    if let Some((body, etag)) = cached_package(&cache_key, &stamp) {
        return Ok(json_response(body, etag, None, if_none_match));
    }

    // A bounded number of builds run at once. Waiting here happens inside the
    // request, so it is cancelled with it; only a build that holds a slot is
    // ever detached.
    let permit = tokio::time::timeout(PACKAGE_BUILD_SLOT_WAIT, BUILD_SLOTS.clone().acquire_owned())
        .await
        .map_err(|_| AppError::Busy)?
        .map_err(|error| anyhow::anyhow!("package build slots closed: {error}"))?;

    // The build runs in its own task, which owns the key's slot and the build
    // permit. A requester that disconnects or hits the request timeout no
    // longer cancels it: the build finishes and caches, and the requests
    // queued on the key read that entry instead of each starting from zero.
    let task_state = state.clone();
    let task_params = params.clone();
    let task_key = cache_key.clone();
    let build = tokio::spawn(async move {
        // Dropped in reverse order: the lock is released before the slot
        // checks whether it was the last holder of the map entry.
        let _slot = slot;
        let _building = building;
        let _permit = permit;
        let built = build_package(&task_state, &task_params, include).await;
        if let Ok((body, etag)) = &built {
            store_package(&task_key, stamp, body.clone(), etag.clone());
        }
        built
    });
    let (body, etag) = build
        .await
        .map_err(|error| anyhow::anyhow!("package build task failed: {error}"))??;
    Ok(json_response(body, etag, None, if_none_match))
}

async fn build_package(
    state: &AppState,
    params: &MeetPackageParams,
    include: PackageInclude,
) -> Result<(Bytes, HeaderValue), AppError> {
    record_build(&params.meet);

    // The meet row, schedule, roster, and this meet's own results depend only
    // on the meet name, so they run concurrently.
    let (meet, schedule_rows, athletes, meet_results) = tokio::try_join!(
        sqlx::query_as::<_, PackageMeet>(MEET_SQL)
            .bind(&params.meet)
            .fetch_one(&state.db),
        sqlx::query_as::<_, ScheduleRow>(SCHEDULE_SQL)
            .bind(&params.meet)
            .fetch_all(&state.db),
        sqlx::query_as::<_, AthletePackageRow>(ATHLETES_SQL)
            .bind(&params.meet)
            .fetch_all(&state.db),
        sqlx::query_as::<_, PackageLiftingResult>(MEET_RESULTS_SQL)
            .bind(&params.meet)
            .fetch_all(&state.db),
    )?;

    let athlete_names: Vec<String> = athletes
        .iter()
        .map(|athlete| athlete.name.clone())
        .collect();
    let normalized_athlete_names: Vec<String> = athlete_names
        .iter()
        .map(|name| normalize_name(name))
        .collect();

    let cutoff_date = params.history_cutoff_date.as_deref();
    let want_history =
        cutoff_date.is_some() && include.needs_history() && !athlete_names.is_empty();
    let want_bests = cutoff_date.is_some() && include.year_bests && !athlete_names.is_empty();

    // History rows (for recent results and estimates) and the year-bests
    // aggregate are independent of each other; each is skipped when no
    // requested section needs it.
    let (history_rows, bests_rows) = tokio::try_join!(
        async {
            if want_history {
                sqlx::query_as::<_, PackageLiftingResult>(ATHLETE_HISTORY_SQL)
                    .bind(&normalized_athlete_names)
                    .bind(cutoff_date)
                    .fetch_all(&state.db)
                    .await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if want_bests {
                sqlx::query_as::<_, YearBestsByNameRow>(YEAR_BESTS_BY_NAME_SQL)
                    .bind(&normalized_athlete_names)
                    .bind(cutoff_date)
                    .fetch_all(&state.db)
                    .await
            } else {
                Ok(Vec::new())
            }
        },
    )?;

    let recent_results_by_name = include.recent_results.then(|| {
        if cutoff_date.is_some() {
            build_recent_results_by_name(&athlete_names, &history_rows)
        } else {
            BTreeMap::new()
        }
    });
    let year_bests_by_name = include.year_bests.then(|| {
        if want_bests {
            year_bests_from_rows(&athlete_names, bests_rows)
        } else {
            BTreeMap::new()
        }
    });

    let package_athletes: Vec<PackageAthlete> =
        athletes.into_iter().map(PackageAthlete::from).collect();
    let attempt_estimates = include
        .attempt_estimates
        .then(|| build_attempt_estimates(&package_athletes, &history_rows));

    let package = MeetPackage {
        meet,
        schedule: build_schedule(schedule_rows),
        athletes: package_athletes,
        meet_results,
        attempt_estimates,
        year_bests_by_name,
        recent_results_by_name,
    };

    let body = Bytes::from(serde_json::to_vec(&package).map_err(anyhow::Error::from)?);
    let etag = strong_etag(&body);
    Ok((body, etag))
}

fn build_schedule(rows: Vec<ScheduleRow>) -> Vec<PackageScheduleDay> {
    let mut days: Vec<PackageScheduleDay> = Vec::new();

    for row in rows {
        let day_index = days.iter().position(|day| day.date == row.date);
        let day_index = match day_index {
            Some(index) => index,
            None => {
                days.push(PackageScheduleDay {
                    date: row.date.clone(),
                    sessions: Vec::new(),
                });
                days.len() - 1
            }
        };

        let sessions = &mut days[day_index].sessions;
        let session_index = sessions
            .iter()
            .position(|session| session.session_id == row.session_id);
        let session_index = match session_index {
            Some(index) => index,
            None => {
                sessions.push(PackageScheduleSession {
                    session_id: row.session_id,
                    start_time: row.start_time.clone(),
                    weigh_in_time: row.weigh_in_time.clone(),
                    platforms: Vec::new(),
                });
                sessions.len() - 1
            }
        };

        sessions[session_index]
            .platforms
            .push(PackageSchedulePlatform {
                platform: row.platform,
                weight_class: row.weight_class,
            });
    }

    days
}

/// History rows bucketed by [`normalize_name`] of the lifter's name, in the
/// order they arrived from Postgres.
type HistoryByName<'a> = HashMap<String, Vec<&'a PackageLiftingResult>>;

/// Normalizes each history row's name exactly once.
///
/// The per-athlete lookup used to re-scan and re-normalize every history row,
/// which is `athletes x history` normalizations and String allocations per
/// cache miss. Bucketing first makes it `history` normalizations plus one hash
/// lookup per athlete. Rows keep their arrival order inside a bucket, which is
/// the order the old `filter` produced, so every downstream average, best, and
/// make-rate sees the same sequence of rows.
fn index_history_by_name(history_rows: &[PackageLiftingResult]) -> HistoryByName<'_> {
    let mut by_name: HistoryByName<'_> = HashMap::new();
    for row in history_rows {
        by_name
            .entry(normalize_name(&row.name))
            .or_default()
            .push(row);
    }
    by_name
}

fn build_attempt_estimates(
    athletes: &[PackageAthlete],
    history_rows: &[PackageLiftingResult],
) -> Vec<PackageAttemptEstimateSession> {
    let history_by_name = index_history_by_name(history_rows);
    let mut sessions: Vec<PackageAttemptEstimateSession> = Vec::new();

    for athlete in athletes {
        let Some(session) = athlete.session.as_ref() else {
            continue;
        };

        let session_exists = sessions.iter().any(|estimate_session| {
            estimate_session.session_number == session.session_number
                && estimate_session.platform == session.session_platform
        });

        if !session_exists {
            sessions.push(PackageAttemptEstimateSession {
                session_number: session.session_number,
                platform: session.session_platform.clone(),
                date: session.date.clone(),
                start_time: session.start_time.clone(),
                weigh_in_time: session.weigh_in_time.clone(),
                estimates: Vec::new(),
            });
        }
    }

    for session in &mut sessions {
        let session_athletes: Vec<&PackageAthlete> = athletes
            .iter()
            .filter(|athlete| {
                athlete.session.as_ref().is_some_and(|athlete_session| {
                    athlete_session.session_number == session.session_number
                        && athlete_session.session_platform == session.platform
                })
            })
            .collect();
        session.estimates = build_session_attempt_estimates(&session_athletes, &history_by_name);
    }

    sessions.sort_by(|a, b| {
        a.session_number
            .total_cmp(&b.session_number)
            .then_with(|| a.platform.cmp(&b.platform))
    });
    sessions
}

/// Attempt-estimator coefficients. Each one appeared as a bare literal at two
/// or three call sites, so a tuning change had to be applied in every copy;
/// they are declared once here with their unit.
///
/// An opener is planned at 93% of the reference lift (historical best, or the
/// declared entry total when there is no history). An entry total is split
/// 43% snatch / 57% clean & jerk. With no history to average, attempts step by
/// the default jump for the lift.
const OPENER_SHARE_OF_BEST: f64 = 0.93;
const SNATCH_SHARE_OF_TOTAL: f64 = 0.43;
const CJ_SHARE_OF_TOTAL: f64 = 0.57;
const DEFAULT_SNATCH_JUMP_KG: f64 = 3.0;
const DEFAULT_CJ_JUMP_KG: f64 = 4.0;

fn build_session_attempt_estimates(
    athletes: &[&PackageAthlete],
    history_by_name: &HistoryByName<'_>,
) -> Vec<PackageAttemptEstimate> {
    let mut temp_estimates = Vec::with_capacity(athletes.len());

    for athlete in athletes {
        let history: &[&PackageLiftingResult] = history_by_name
            .get(&normalize_name(&athlete.name))
            .map_or(&[], Vec::as_slice);

        let best_snatch = history
            .iter()
            .filter_map(|row| snatch_best(row))
            .reduce(f64::max);
        let best_cj = history
            .iter()
            .filter_map(|row| cj_best(row))
            .reduce(f64::max);
        let avg_snatch_increase = calculate_average_increase(history, LiftType::Snatch);
        let avg_cj_increase = calculate_average_increase(history, LiftType::CleanAndJerk);
        let (snatch_make_rate, cj_make_rate) = calculate_make_rates(history);

        temp_estimates.push(TempAttemptEstimate {
            athlete: (*athlete).clone(),
            history,
            best_snatch,
            best_cj,
            avg_snatch_increase,
            avg_cj_increase,
            snatch_make_rate,
            cj_make_rate,
        });
    }

    let session_avg_snatch = session_average_increase(
        temp_estimates
            .iter()
            .filter(|estimate| estimate.best_snatch.is_some())
            .map(|estimate| estimate.avg_snatch_increase),
        DEFAULT_SNATCH_JUMP_KG,
    );
    let session_avg_cj = session_average_increase(
        temp_estimates
            .iter()
            .filter(|estimate| estimate.best_cj.is_some())
            .map(|estimate| estimate.avg_cj_increase),
        DEFAULT_CJ_JUMP_KG,
    );

    let mut estimates: Vec<PackageAttemptEstimate> = temp_estimates
        .into_iter()
        .map(|estimate| {
            let snatch = estimate_lift(
                estimate.best_snatch,
                estimate.avg_snatch_increase,
                session_avg_snatch,
                estimate.athlete.entry_total,
                SNATCH_SHARE_OF_TOTAL,
                DEFAULT_SNATCH_JUMP_KG,
                estimate.snatch_make_rate,
            );
            let clean_and_jerk = estimate_lift(
                estimate.best_cj,
                estimate.avg_cj_increase,
                session_avg_cj,
                estimate.athlete.entry_total,
                CJ_SHARE_OF_TOTAL,
                DEFAULT_CJ_JUMP_KG,
                estimate.cj_make_rate,
            );

            PackageAttemptEstimate {
                athlete_id: estimate.athlete.member_id.clone(),
                athlete_name: estimate.athlete.name,
                weight_class: estimate.athlete.weight_class,
                entry_total: estimate.athlete.entry_total,
                history_result_count: estimate.history.len(),
                snatch,
                clean_and_jerk,
            }
        })
        .collect();

    calculate_attempts_out(&mut estimates);
    estimates.sort_by(|a, b| {
        a.snatch
            .attempts_out
            .cmp(&b.snatch.attempts_out)
            .then_with(|| {
                a.clean_and_jerk
                    .attempts_out
                    .cmp(&b.clean_and_jerk.attempts_out)
            })
            .then_with(|| a.athlete_name.cmp(&b.athlete_name))
    });
    estimates
}

fn estimate_lift(
    historical_best: Option<f64>,
    athlete_average_increase: AttemptIncrease,
    session_average_increase: AttemptIncrease,
    entry_total: f64,
    total_ratio: f64,
    default_jump: f64,
    make_rate: f64,
) -> PackageLiftAttemptEstimate {
    if let Some(best) = historical_best {
        let first_attempt = (best * OPENER_SHARE_OF_BEST).round();
        let second_attempt = first_attempt + athlete_average_increase.first_to_second;
        let third_attempt = second_attempt + athlete_average_increase.second_to_third;
        return PackageLiftAttemptEstimate {
            attempts: vec![first_attempt, second_attempt, third_attempt],
            attempts_out: 0,
            average_increase: athlete_average_increase,
            make_rate,
            historical_best: Some(best),
            source: AttemptEstimateSource::History,
        };
    }

    if entry_total > 0.0 {
        let estimated_total = (entry_total * OPENER_SHARE_OF_BEST).round();
        let first_attempt = (estimated_total * total_ratio).round();
        let second_attempt = first_attempt + session_average_increase.first_to_second;
        let third_attempt = second_attempt + session_average_increase.second_to_third;
        return PackageLiftAttemptEstimate {
            attempts: vec![first_attempt, second_attempt, third_attempt],
            attempts_out: 0,
            average_increase: session_average_increase,
            make_rate,
            historical_best: None,
            source: AttemptEstimateSource::EntryTotal,
        };
    }

    unavailable_lift_estimate(default_jump)
}

fn unavailable_lift_estimate(default_jump: f64) -> PackageLiftAttemptEstimate {
    PackageLiftAttemptEstimate {
        attempts: Vec::new(),
        attempts_out: 0,
        average_increase: AttemptIncrease {
            first_to_second: default_jump,
            second_to_third: default_jump,
        },
        make_rate: 0.0,
        historical_best: None,
        source: AttemptEstimateSource::Unavailable,
    }
}

fn calculate_attempts_out(estimates: &mut [PackageAttemptEstimate]) {
    let snatch_attempts = collect_attempts(estimates, LiftType::Snatch);
    let cj_attempts = collect_attempts(estimates, LiftType::CleanAndJerk);

    for estimate in estimates {
        estimate.snatch.attempts_out =
            attempts_out_before_first_attempt(&snatch_attempts, &estimate.athlete_id);
        estimate.clean_and_jerk.attempts_out =
            attempts_out_before_first_attempt(&cj_attempts, &estimate.athlete_id);
    }
}

fn collect_attempts(estimates: &[PackageAttemptEstimate], lift_type: LiftType) -> Vec<AttemptData> {
    let mut attempts = Vec::new();

    for estimate in estimates {
        let lift_attempts = match lift_type {
            LiftType::Snatch => &estimate.snatch.attempts,
            LiftType::CleanAndJerk => &estimate.clean_and_jerk.attempts,
        };

        for (index, weight) in lift_attempts.iter().enumerate() {
            if *weight > 0.0 {
                attempts.push(AttemptData {
                    athlete_id: estimate.athlete_id.clone(),
                    weight: *weight,
                    attempt_number: index + 1,
                });
            }
        }
    }

    attempts.sort_by(|a, b| a.weight.total_cmp(&b.weight));
    attempts
}

fn attempts_out_before_first_attempt(attempts: &[AttemptData], athlete_id: &str) -> usize {
    let Some(first_attempt_index) = attempts
        .iter()
        .position(|attempt| attempt.athlete_id == athlete_id && attempt.attempt_number == 1)
    else {
        return 0;
    };

    let mut attempts_out = 0;
    for index in 0..first_attempt_index {
        attempts_out += 1;
        if index + 1 < first_attempt_index
            && attempts[index].athlete_id == attempts[index + 1].athlete_id
        {
            attempts_out += 1;
        }
    }

    attempts_out
}

fn calculate_average_increase(
    results: &[&PackageLiftingResult],
    lift_type: LiftType,
) -> AttemptIncrease {
    let mut first_to_second = Vec::new();
    let mut second_to_third = Vec::new();

    for result in results {
        let attempts = match lift_type {
            LiftType::Snatch => [result.snatch1, result.snatch2, result.snatch3],
            LiftType::CleanAndJerk => [result.cj1, result.cj2, result.cj3],
        };

        if attempts[0] > 0.0 && attempts[1] > 0.0 {
            first_to_second.push((attempts[1] - attempts[0]).abs());
        }
        if attempts[1] > 0.0 && attempts[2] > 0.0 {
            second_to_third.push((attempts[2] - attempts[1]).abs());
        }
    }

    let default_jump = match lift_type {
        LiftType::Snatch => DEFAULT_SNATCH_JUMP_KG,
        LiftType::CleanAndJerk => DEFAULT_CJ_JUMP_KG,
    };

    AttemptIncrease {
        first_to_second: rounded_average(&first_to_second, default_jump),
        second_to_third: rounded_average(&second_to_third, default_jump),
    }
}

fn session_average_increase(
    increases: impl Iterator<Item = AttemptIncrease>,
    default_jump: f64,
) -> AttemptIncrease {
    let increases: Vec<AttemptIncrease> = increases.collect();
    if increases.is_empty() {
        return AttemptIncrease {
            first_to_second: default_jump,
            second_to_third: default_jump,
        };
    }

    let first_to_second: Vec<f64> = increases
        .iter()
        .map(|increase| increase.first_to_second)
        .collect();
    let second_to_third: Vec<f64> = increases
        .iter()
        .map(|increase| increase.second_to_third)
        .collect();

    AttemptIncrease {
        first_to_second: rounded_average(&first_to_second, default_jump),
        second_to_third: rounded_average(&second_to_third, default_jump),
    }
}

fn rounded_average(values: &[f64], default_value: f64) -> f64 {
    if values.is_empty() {
        return default_value;
    }

    (values.iter().sum::<f64>() / values.len() as f64).round()
}

fn calculate_make_rates(results: &[&PackageLiftingResult]) -> (f64, f64) {
    (
        calculate_lift_make_rate(results, LiftType::Snatch),
        calculate_lift_make_rate(results, LiftType::CleanAndJerk),
    )
}

fn calculate_lift_make_rate(results: &[&PackageLiftingResult], lift_type: LiftType) -> f64 {
    let mut openers_declared = 0;
    let mut openers_made = 0;

    for result in results {
        let opener = match lift_type {
            LiftType::Snatch => result.snatch1,
            LiftType::CleanAndJerk => result.cj1,
        };

        if opener != 0.0 {
            openers_declared += 1;
            if opener > 0.0 {
                openers_made += 1;
            }
        }
    }

    if openers_declared == 0 {
        0.0
    } else {
        openers_made as f64 / openers_declared as f64
    }
}

fn snatch_best(result: &PackageLiftingResult) -> Option<f64> {
    max_successful([
        result.snatch_best,
        result.snatch1,
        result.snatch2,
        result.snatch3,
    ])
}

fn cj_best(result: &PackageLiftingResult) -> Option<f64> {
    max_successful([result.cj_best, result.cj1, result.cj2, result.cj3])
}

fn max_successful(values: [f64; 4]) -> Option<f64> {
    let best = max_positive(values);
    if best > 0.0 { Some(best) } else { None }
}

/// Groups history rows under each requested athlete name. Year bests come from
/// [`year_bests_from_rows`] over a separate aggregate query.
fn build_recent_results_by_name(
    athlete_names: &[String],
    rows: &[PackageLiftingResult],
) -> BTreeMap<String, Vec<PackageLiftingResult>> {
    let requested_by_normalized = requested_names_by_normalized(athlete_names);

    let mut recent_results_by_name: BTreeMap<String, Vec<PackageLiftingResult>> = athlete_names
        .iter()
        .map(|name| (name.clone(), Vec::new()))
        .collect();

    for row in rows {
        // Attribute each result to the requested athlete name(s) it matches once
        // case and whitespace are normalized, so "Anna Mcelderry" picks up rows
        // stored as "Anna McElderry".
        let Some(requested_names) = requested_by_normalized.get(&normalize_name(&row.name)) else {
            continue;
        };

        for requested in requested_names {
            recent_results_by_name
                .entry(requested.clone())
                .or_default()
                .push(row.clone());
        }
    }

    recent_results_by_name
}

/// Builds a lookup from normalized name to the requested display name(s) that
/// normalize to it, so result rows can be attributed back to the caller's names.
fn requested_names_by_normalized(athlete_names: &[String]) -> HashMap<String, Vec<String>> {
    let mut by_normalized: HashMap<String, Vec<String>> = HashMap::new();
    for name in athlete_names {
        by_normalized
            .entry(normalize_name(name))
            .or_default()
            .push(name.clone());
    }
    by_normalized
}

/// Keys the year-bests aggregate rows by every requested athlete name that
/// normalizes to the row's name, defaulting names with no rows to zeros.
fn year_bests_from_rows(
    athlete_names: &[String],
    rows: Vec<YearBestsByNameRow>,
) -> BTreeMap<String, YearBests> {
    let mut bests_by_name: BTreeMap<String, YearBests> = athlete_names
        .iter()
        .map(|name| {
            (
                name.clone(),
                YearBests {
                    best_snatch: 0.0,
                    best_cj: 0.0,
                    best_total: 0.0,
                },
            )
        })
        .collect();

    let requested_by_normalized = requested_names_by_normalized(athlete_names);

    for row in rows {
        let Some(requested_names) = requested_by_normalized.get(&row.name) else {
            continue;
        };
        for requested in requested_names {
            bests_by_name.insert(
                requested.clone(),
                YearBests {
                    best_snatch: row.best_snatch,
                    best_cj: row.best_cj,
                    best_total: row.best_total,
                },
            );
        }
    }

    bests_by_name
}

fn max_positive(values: [f64; 4]) -> f64 {
    values
        .into_iter()
        .filter(|value| *value > 0.0)
        .fold(0.0, f64::max)
}

/// Pre-optimization implementations of [`build_attempt_estimates`] and
/// [`build_session_attempt_estimates`], kept as the regression oracle for
/// `attempt_estimates_match_pre_index_reference`.
///
/// They are the original control flow verbatim: the per-athlete history is
/// found by rescanning and re-normalizing every history row, which is the
/// O(athletes x history) cost `index_history_by_name` removed. Only the
/// container changed (owned row clones -> borrows), which cannot affect
/// output; every comparison, average, sort key, and tie-break is the original.
/// If a future change to the fast path alters ordering or rounding, the
/// equivalence test fails.
#[cfg(test)]
mod reference {
    use super::*;

    struct RefTempEstimate<'a> {
        athlete: PackageAthlete,
        history: Vec<&'a PackageLiftingResult>,
        best_snatch: Option<f64>,
        best_cj: Option<f64>,
        avg_snatch_increase: AttemptIncrease,
        avg_cj_increase: AttemptIncrease,
        snatch_make_rate: f64,
        cj_make_rate: f64,
    }

    pub(super) fn build_attempt_estimates_reference(
        athletes: &[PackageAthlete],
        history_rows: &[PackageLiftingResult],
    ) -> Vec<PackageAttemptEstimateSession> {
        let mut sessions: Vec<PackageAttemptEstimateSession> = Vec::new();

        for athlete in athletes {
            let Some(session) = athlete.session.as_ref() else {
                continue;
            };

            let session_exists = sessions.iter().any(|estimate_session| {
                estimate_session.session_number == session.session_number
                    && estimate_session.platform == session.session_platform
            });

            if !session_exists {
                sessions.push(PackageAttemptEstimateSession {
                    session_number: session.session_number,
                    platform: session.session_platform.clone(),
                    date: session.date.clone(),
                    start_time: session.start_time.clone(),
                    weigh_in_time: session.weigh_in_time.clone(),
                    estimates: Vec::new(),
                });
            }
        }

        for session in &mut sessions {
            let session_athletes: Vec<PackageAthlete> = athletes
                .iter()
                .filter(|athlete| {
                    athlete.session.as_ref().is_some_and(|athlete_session| {
                        athlete_session.session_number == session.session_number
                            && athlete_session.session_platform == session.platform
                    })
                })
                .cloned()
                .collect();
            session.estimates = build_session_reference(&session_athletes, history_rows);
        }

        sessions.sort_by(|a, b| {
            a.session_number
                .total_cmp(&b.session_number)
                .then_with(|| a.platform.cmp(&b.platform))
        });
        sessions
    }

    fn build_session_reference(
        athletes: &[PackageAthlete],
        history_rows: &[PackageLiftingResult],
    ) -> Vec<PackageAttemptEstimate> {
        let mut temp_estimates = Vec::new();

        for athlete in athletes {
            let normalized_name = normalize_name(&athlete.name);
            let history: Vec<&PackageLiftingResult> = history_rows
                .iter()
                .filter(|row| normalize_name(&row.name) == normalized_name)
                .collect();

            let best_snatch = history
                .iter()
                .filter_map(|row| snatch_best(row))
                .reduce(f64::max);
            let best_cj = history
                .iter()
                .filter_map(|row| cj_best(row))
                .reduce(f64::max);
            let avg_snatch_increase = calculate_average_increase(&history, LiftType::Snatch);
            let avg_cj_increase = calculate_average_increase(&history, LiftType::CleanAndJerk);
            let (snatch_make_rate, cj_make_rate) = calculate_make_rates(&history);

            temp_estimates.push(RefTempEstimate {
                athlete: athlete.clone(),
                history,
                best_snatch,
                best_cj,
                avg_snatch_increase,
                avg_cj_increase,
                snatch_make_rate,
                cj_make_rate,
            });
        }

        let session_avg_snatch = session_average_increase(
            temp_estimates
                .iter()
                .filter(|estimate| estimate.best_snatch.is_some())
                .map(|estimate| estimate.avg_snatch_increase),
            DEFAULT_SNATCH_JUMP_KG,
        );
        let session_avg_cj = session_average_increase(
            temp_estimates
                .iter()
                .filter(|estimate| estimate.best_cj.is_some())
                .map(|estimate| estimate.avg_cj_increase),
            DEFAULT_CJ_JUMP_KG,
        );

        let mut estimates: Vec<PackageAttemptEstimate> = temp_estimates
            .into_iter()
            .map(|estimate| {
                let snatch = estimate_lift(
                    estimate.best_snatch,
                    estimate.avg_snatch_increase,
                    session_avg_snatch,
                    estimate.athlete.entry_total,
                    SNATCH_SHARE_OF_TOTAL,
                    DEFAULT_SNATCH_JUMP_KG,
                    estimate.snatch_make_rate,
                );
                let clean_and_jerk = estimate_lift(
                    estimate.best_cj,
                    estimate.avg_cj_increase,
                    session_avg_cj,
                    estimate.athlete.entry_total,
                    CJ_SHARE_OF_TOTAL,
                    DEFAULT_CJ_JUMP_KG,
                    estimate.cj_make_rate,
                );

                PackageAttemptEstimate {
                    athlete_id: estimate.athlete.member_id.clone(),
                    athlete_name: estimate.athlete.name,
                    weight_class: estimate.athlete.weight_class,
                    entry_total: estimate.athlete.entry_total,
                    history_result_count: estimate.history.len(),
                    snatch,
                    clean_and_jerk,
                }
            })
            .collect();

        calculate_attempts_out(&mut estimates);
        estimates.sort_by(|a, b| {
            a.snatch
                .attempts_out
                .cmp(&b.snatch.attempts_out)
                .then_with(|| {
                    a.clean_and_jerk
                        .attempts_out
                        .cmp(&b.clean_and_jerk.attempts_out)
                })
                .then_with(|| a.athlete_name.cmp(&b.athlete_name))
        });
        estimates
    }
}

impl From<AthletePackageRow> for PackageAthlete {
    fn from(row: AthletePackageRow) -> Self {
        let session = match (row.session_number, row.session_platform) {
            (Some(number), Some(platform)) => Some(PackageAthleteSession {
                session_number: number,
                session_platform: platform,
                date: row.date.clone(),
                start_time: row.start_time.clone(),
                weigh_in_time: row.weigh_in_time.clone(),
            }),
            _ => None,
        };

        Self {
            member_id: row.member_id,
            name: row.name,
            age: row.age,
            club: row.club,
            wso: row.wso,
            gender: row.gender,
            weight_class: row.weight_class,
            entry_total: row.entry_total,
            adaptive: row.adaptive,
            session,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_estimates_use_entry_total_when_history_is_missing() {
        let athletes = vec![PackageAthlete {
            member_id: "12345".to_string(),
            name: "Kyle Schulman".to_string(),
            age: 27.0,
            club: "Vardanian Weightlifting".to_string(),
            wso: None,
            gender: "Male".to_string(),
            weight_class: "+110".to_string(),
            entry_total: 365.0,
            adaptive: false,
            session: Some(PackageAthleteSession {
                session_number: 45.0,
                session_platform: "Red".to_string(),
                date: Some("2026-06-20".to_string()),
                start_time: Some("08:00:00".to_string()),
                weigh_in_time: Some("06:00:00".to_string()),
            }),
        }];

        let sessions = build_attempt_estimates(&athletes, &[]);

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_number, 45.0);
        assert_eq!(sessions[0].platform, "Red");
        assert_eq!(sessions[0].estimates.len(), 1);

        let estimate = &sessions[0].estimates[0];
        assert_eq!(estimate.athlete_name, "Kyle Schulman");
        assert_eq!(estimate.history_result_count, 0);
        assert_eq!(estimate.snatch.attempts, vec![146.0, 149.0, 152.0]);
        assert_eq!(estimate.clean_and_jerk.attempts, vec![193.0, 197.0, 201.0]);
        assert!(matches!(
            estimate.snatch.source,
            AttemptEstimateSource::EntryTotal
        ));
        assert!(matches!(
            estimate.clean_and_jerk.source,
            AttemptEstimateSource::EntryTotal
        ));
    }

    /// Fixtures for the equivalence test. Names are deliberately adversarial
    /// for the normalized-name index: duplicates, case-only and whitespace-only
    /// differences, an athlete with no history at all, and history rows that
    /// match no athlete.
    fn equivalence_athlete(
        member_id: &str,
        name: &str,
        entry_total: f64,
        session_number: f64,
        platform: &str,
    ) -> PackageAthlete {
        PackageAthlete {
            member_id: member_id.to_string(),
            name: name.to_string(),
            age: 24.0,
            club: "Test Barbell".to_string(),
            wso: Some("Test WSO".to_string()),
            gender: "Female".to_string(),
            weight_class: "71".to_string(),
            entry_total,
            adaptive: false,
            session: Some(PackageAthleteSession {
                session_number,
                session_platform: platform.to_string(),
                date: Some("2026-06-20".to_string()),
                start_time: Some("08:00:00".to_string()),
                weigh_in_time: Some("06:00:00".to_string()),
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn equivalence_result(
        id: i64,
        name: &str,
        date: &str,
        snatches: [f64; 3],
        snatch_best: f64,
        cjs: [f64; 3],
        cj_best: f64,
    ) -> PackageLiftingResult {
        PackageLiftingResult {
            id,
            event_id: format!("event-{id}"),
            federation: "USAW".to_string(),
            meet: format!("Meet {id}"),
            date: date.to_string(),
            name: name.to_string(),
            age: "Open".to_string(),
            body_weight: 70.0,
            snatch1: snatches[0],
            snatch2: snatches[1],
            snatch3: snatches[2],
            snatch_best,
            cj1: cjs[0],
            cj2: cjs[1],
            cj3: cjs[2],
            cj_best,
            total: snatch_best + cj_best,
            adaptive: false,
        }
    }

    /// The normalized-name index in [`build_attempt_estimates`] is a pure
    /// performance change, so its output must be byte-for-byte what the
    /// pre-index implementation produced -- same sessions, same order, same
    /// tie-breaks, same rounding, same `history_result_count`. This compares
    /// the serialized JSON of both implementations over a fixture built to
    /// exercise every way the two could diverge.
    #[test]
    fn attempt_estimates_match_pre_index_reference() {
        let athletes = vec![
            // Same lifter spelled three ways: exact, upper-case, and with
            // collapsible whitespace. All three must find the same history.
            equivalence_athlete("1", "Alexander Nordstrom", 300.0, 1.0, "Red"),
            equivalence_athlete("2", "ALEXANDER  NORDSTROM", 300.0, 1.0, "Red"),
            equivalence_athlete("3", "  alexander nordstrom  ", 295.0, 2.0, "Blue"),
            // Two distinct athletes sharing a display name: the estimates must
            // still be emitted once per athlete, ordered by the same tie-break.
            equivalence_athlete("4", "Jordan Lee", 250.0, 1.0, "Red"),
            equivalence_athlete("5", "Jordan Lee", 250.0, 1.0, "Red"),
            // No history anywhere in the fixture -> entry-total path.
            equivalence_athlete("6", "Ghost Lifter", 210.0, 1.0, "Red"),
            // No history and no entry total -> unavailable path.
            equivalence_athlete("7", "Zero Total", 0.0, 2.0, "Blue"),
            // Same session number, different platform: two distinct sessions.
            equivalence_athlete("8", "Platform Split", 240.0, 1.0, "Blue"),
            // Unsessioned athletes are skipped entirely.
            PackageAthlete {
                session: None,
                ..equivalence_athlete("9", "No Session", 260.0, 3.0, "Red")
            },
        ];

        let history = vec![
            // Order matters: averages and make rates walk the rows in arrival
            // order, so the index must preserve it inside each name bucket.
            equivalence_result(
                1,
                "alexander nordstrom",
                "2025-03-01",
                [100.0, 104.0, -108.0],
                104.0,
                [125.0, 130.0, 134.0],
                134.0,
            ),
            equivalence_result(
                2,
                "Alexander   Nordstrom",
                "2025-07-01",
                [-101.0, 106.0, 110.0],
                110.0,
                [-128.0, 132.0, 0.0],
                132.0,
            ),
            equivalence_result(
                3,
                "ALEXANDER NORDSTROM",
                "2024-11-01",
                [98.0, 0.0, 0.0],
                98.0,
                [0.0, 0.0, 0.0],
                0.0,
            ),
            equivalence_result(
                4,
                "Jordan Lee",
                "2025-05-01",
                [80.0, 84.0, 87.0],
                87.0,
                [100.0, 105.0, -109.0],
                105.0,
            ),
            // Matches no athlete in the roster -- must be bucketed and ignored,
            // never folded into someone else's averages.
            equivalence_result(
                5,
                "Unrelated Person",
                "2025-05-01",
                [200.0, 205.0, 210.0],
                210.0,
                [240.0, 245.0, 250.0],
                250.0,
            ),
            equivalence_result(
                6,
                "  PLATFORM   split ",
                "2025-01-01",
                [90.0, 0.0, 95.0],
                95.0,
                [115.0, 0.0, 0.0],
                115.0,
            ),
        ];

        let optimized = build_attempt_estimates(&athletes, &history);
        let reference = reference::build_attempt_estimates_reference(&athletes, &history);

        assert_eq!(
            serde_json::to_string(&optimized).expect("optimized estimates serialize"),
            serde_json::to_string(&reference).expect("reference estimates serialize"),
        );

        // Guard against the fixture silently degenerating into "both empty".
        assert_eq!(optimized.len(), 3, "expected three distinct sessions");
        assert!(
            optimized.iter().any(|session| session
                .estimates
                .iter()
                .any(|estimate| matches!(estimate.snatch.source, AttemptEstimateSource::History))),
            "fixture must exercise the history path"
        );

        // The empty-history and empty-roster edges go through the same index.
        assert_eq!(
            serde_json::to_string(&build_attempt_estimates(&athletes, &[])).unwrap(),
            serde_json::to_string(&reference::build_attempt_estimates_reference(
                &athletes,
                &[]
            ))
            .unwrap(),
        );
        assert_eq!(
            serde_json::to_string(&build_attempt_estimates(&[], &history)).unwrap(),
            serde_json::to_string(&reference::build_attempt_estimates_reference(&[], &history))
                .unwrap(),
        );
    }
}
