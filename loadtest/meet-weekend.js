// Meet-weekend load test for the MeetCal API.
//
// Simulates the request mix the shipped app (>= 6.2.0) actually produces over
// a meet weekend: a few hundred users open the app clustered around session
// changes. Three scenarios run at once:
//
//   meet_weekend_spike  ramps to PEAK_VUS running the common flow: app open
//                       (/meets, /meets/details, /meets/schedule,
//                       /meets/athletes-sessions), start-list bests
//                       (POST /lifting-results/bests), the attempt estimator
//                       (POST /lifting-results/recent), one club stats page,
//                       one athlete search, then browsing.
//   offline_package     ~PACKAGE_RATE of PEAK_VUS run the offline download:
//                       GET /meets/package (ETag captured) followed by
//                       POST /lifting-results/by-names in batches of <= 40,
//                       then re-validate the package with If-None-Match and
//                       expect 304.
//   saved_sessions      optional; only when CLERK_JWT is set. Signed-in users
//                       polling GET /users/me/saved-sessions.
//
// Every request carries `X-MeetCal-App` so the strict (fail-closed) validation
// path is exercised, the same one the current app hits.
//
// Usage:
//   k6 run loadtest/meet-weekend.js
//   BASE_URL=https://staging-api.example.test MEET="2026 Ohio WSO Championships" \
//     PEAK_VUS=100 k6 run loadtest/meet-weekend.js
//
// Point BASE_URL at a STAGING copy, never production. Defaults hit a local
// server. Athlete names and clubs are seeded from the meet's roster call in
// setup(), so any meet with a start list works.

import http from 'k6/http';
import { check, sleep, group } from 'k6';
import { Trend, Rate, Counter } from 'k6/metrics';

const BASE = __ENV.BASE_URL || 'http://127.0.0.1:3000';
const MEET = __ENV.MEET || '2026 USA Weightlifting National Championships, Powered by Rogue Fitness';
const APP_VERSION = __ENV.APP_VERSION || '6.2.0';
const PEAK_VUS = Number(__ENV.PEAK_VUS || 100);
// Fraction of users that trigger the heavy offline package download.
const PACKAGE_RATE = Number(__ENV.PACKAGE_RATE || 0.2);
// The app sends today - 2 years as the package history cutoff; override to pin it.
const HISTORY_CUTOFF = __ENV.HISTORY_CUTOFF || twoYearsAgoIso();
// Optional Clerk JWT for the signed-in scenario. Leave unset to skip it.
const CLERK_JWT = __ENV.CLERK_JWT || '';
// The app batches by-names lookups in groups of <= 40 names.
const BY_NAMES_BATCH = 40;
// Bests/recent calls carry a handful of names (one session's worth).
const SESSION_NAMES = 12;

const packageTrend = new Trend('package_duration', true);
const byNamesTrend = new Trend('by_names_duration', true);
const revalidate304 = new Rate('package_revalidate_304');
const flowErrors = new Rate('flow_errors');
const byNamesBatches = new Counter('by_names_batches');

const PACKAGE_VUS = Math.max(1, Math.ceil(PEAK_VUS * PACKAGE_RATE));
const BROWSE_VUS = Math.max(1, PEAK_VUS - PACKAGE_VUS);

function twoYearsAgoIso() {
  const d = new Date();
  d.setUTCFullYear(d.getUTCFullYear() - 2);
  return d.toISOString().slice(0, 10);
}

function ramp(peak) {
  return [
    { duration: '30s', target: Math.ceil(peak / 2) }, // doors open
    { duration: '1m', target: peak }, // peak concurrency
    { duration: '2m', target: peak }, // sustain through a session
    { duration: '30s', target: 0 }, // wind down
  ];
}

const scenarios = {
  meet_weekend_spike: {
    executor: 'ramping-vus',
    exec: 'browseFlow',
    startVUs: 0,
    stages: ramp(BROWSE_VUS),
    gracefulRampDown: '10s',
  },
  offline_package: {
    executor: 'ramping-vus',
    exec: 'packageFlow',
    startVUs: 0,
    stages: ramp(PACKAGE_VUS),
    gracefulRampDown: '30s',
  },
};
if (CLERK_JWT) {
  scenarios.saved_sessions = {
    executor: 'constant-vus',
    exec: 'savedSessionsFlow',
    vus: Math.max(1, Math.ceil(PEAK_VUS / 10)),
    duration: '4m',
  };
}

export const options = {
  scenarios,
  thresholds: {
    // Fail the run if these are breached, so it's CI/gate-friendly.
    http_req_failed: ['rate<0.01'], // <1% HTTP errors overall
    flow_errors: ['rate<0.01'],
    'http_req_duration{kind:light}': ['p(95)<800'], // light GETs snappy
    'http_req_duration{kind:bests}': ['p(95)<1000'],
    'http_req_duration{kind:recent}': ['p(95)<1000'],
    'http_req_duration{kind:club_stats}': ['p(95)<1000'],
    'http_req_duration{kind:search}': ['p(95)<1000'],
    'http_req_duration{kind:revalidate}': ['p(95)<300'], // 304 is a hash compare
    package_duration: ['p(95)<5000'], // heavy package under 5s p95
    by_names_duration: ['p(95)<1500'], // one <=40-name batch
    package_revalidate_304: ['rate>0.99'], // ETag round trip must hit
  },
};

function headers(extra) {
  const h = { 'X-MeetCal-App': APP_VERSION, Accept: 'application/json' };
  if (CLERK_JWT) h.Authorization = `Bearer ${CLERK_JWT}`;
  return Object.assign(h, extra || {});
}

function getJson(path, kind, extraHeaders, opts) {
  const res = http.get(`${BASE}${path}`, Object.assign({ headers: headers(extraHeaders), tags: { kind } }, opts || {}));
  const ok = check(res, { [`200 ${kind}`]: (r) => r.status === 200 });
  flowErrors.add(!ok);
  return res;
}

function postJson(path, body, kind) {
  const res = http.post(`${BASE}${path}`, JSON.stringify(body), {
    headers: headers({ 'Content-Type': 'application/json' }),
    tags: { kind },
  });
  const ok = check(res, { [`200 ${kind}`]: (r) => r.status === 200 });
  flowErrors.add(!ok);
  return res;
}

function pick(list, n) {
  if (!list.length) return [];
  const start = Math.floor(Math.random() * list.length);
  const out = [];
  for (let i = 0; i < Math.min(n, list.length); i += 1) {
    out.push(list[(start + i) % list.length]);
  }
  return out;
}

// Seed names + clubs from the roster once; every VU reuses the result.
export function setup() {
  const meet = encodeURIComponent(MEET);
  const res = http.get(`${BASE}/meets/athletes-sessions?meet=${meet}`, { headers: headers() });
  if (res.status !== 200) {
    throw new Error(`roster call failed (${res.status}) for meet "${MEET}" at ${BASE}: ${res.body}`);
  }
  const roster = res.json();
  const names = [];
  const clubs = new Set();
  for (const a of roster) {
    if (a.name) names.push(a.name);
    if (a.club) clubs.add(a.club);
  }
  if (names.length === 0) {
    throw new Error(`meet "${MEET}" has no athletes; pick a meet with a start list`);
  }
  return { names, clubs: Array.from(clubs) };
}

// Scenario 1: the common flow, no offline download.
export function browseFlow(data) {
  const meet = encodeURIComponent(MEET);

  // 1. App open: health + meet list + details + schedule + roster.
  group('open', () => {
    getJson('/health', 'light');
    getJson('/meets', 'light');
    getJson(`/meets/details?meet=${meet}`, 'light');
    getJson(`/meets/schedule?meet=${meet}`, 'light');
    getJson(`/meets/athletes-sessions?meet=${meet}`, 'light');
  });
  sleep(Math.random() * 2);

  // 2. Start list: bests for one session's athletes; then the attempt
  //    estimator asks for their recent results.
  group('start_list', () => {
    const names = pick(data.names, SESSION_NAMES);
    postJson('/lifting-results/bests', { names, cutoff_date: HISTORY_CUTOFF }, 'bests');
    postJson('/lifting-results/recent', { names: pick(names, 3), cutoff_date: HISTORY_CUTOFF }, 'recent');
  });
  sleep(1 + Math.random() * 3);

  // 3. One club stats page and one athlete search.
  group('club_and_search', () => {
    if (data.clubs.length) {
      const club = encodeURIComponent(pick(data.clubs, 1)[0]);
      getJson(`/clubs/meet-stats?club=${club}&meet=${meet}`, 'club_stats');
    }
    const q = encodeURIComponent(pick(data.names, 1)[0].split(' ')[0]);
    getJson(`/search?query=${q}`, 'search');
  });
  sleep(Math.random() * 2);

  // 4. Browsing comp data / athletes.
  group('browse', () => {
    getJson('/data/wso', 'light');
    getJson(`/meets/athletes?meet=${meet}`, 'light');
  });
  sleep(1 + Math.random() * 4);
}

// Scenario 2: the offline download. Package, then by-names batches for every
// athlete on the roster, then an ETag revalidate that must come back 304.
export function packageFlow(data) {
  const meet = encodeURIComponent(MEET);
  let etag = null;

  group('package', () => {
    const res = http.get(
      `${BASE}/meets/package?meet=${meet}&history_cutoff_date=${HISTORY_CUTOFF}`,
      { headers: headers(), tags: { kind: 'package' }, timeout: '30s' },
    );
    packageTrend.add(res.timings.duration);
    const ok = check(res, {
      'package 200': (r) => r.status === 200,
      'package has ETag': (r) => !!r.headers.ETag || !!r.headers.Etag,
    });
    flowErrors.add(!ok);
    etag = res.headers.ETag || res.headers.Etag || null;
  });

  group('by_names_batches', () => {
    for (let i = 0; i < data.names.length; i += BY_NAMES_BATCH) {
      const names = data.names.slice(i, i + BY_NAMES_BATCH);
      const res = postJson('/lifting-results/by-names', { names }, 'by_names');
      byNamesTrend.add(res.timings.duration);
      byNamesBatches.add(1);
    }
  });
  sleep(1 + Math.random() * 2);

  // Re-open the app later: the package is revalidated, not re-downloaded.
  group('package_revalidate', () => {
    if (!etag) {
      revalidate304.add(false);
      return;
    }
    for (let i = 0; i < 3; i += 1) {
      const res = http.get(
        `${BASE}/meets/package?meet=${meet}&history_cutoff_date=${HISTORY_CUTOFF}`,
        { headers: headers({ 'If-None-Match': etag }), tags: { kind: 'revalidate' }, timeout: '30s' },
      );
      const hit = res.status === 304;
      revalidate304.add(hit);
      // 200 with a fresh body is legitimate if the data changed mid-run; only
      // a non-2xx/304 is a flow error.
      flowErrors.add(!(hit || res.status === 200));
      if (res.status === 200) etag = res.headers.ETag || res.headers.Etag || etag;
      sleep(0.5 + Math.random());
    }
  });
  sleep(2 + Math.random() * 4);
}

// Scenario 3 (optional): a signed-in user's saved sessions poll.
export function savedSessionsFlow() {
  getJson('/users/me/saved-sessions', 'light');
  sleep(3 + Math.random() * 5);
}

export default function (data) {
  browseFlow(data);
}
