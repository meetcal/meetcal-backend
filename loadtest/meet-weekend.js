// Meet-weekend load test for the MeetCal API.
//
// Simulates the real traffic shape: a seasonal spike where a few hundred users
// open the app over one weekend, clustered around session changes. It ramps to
// ~100 concurrent virtual users running the common flow (open -> schedule ->
// start list -> browse), with ~20% also triggering the heavy /meets/package
// offline download.
//
// Usage:
//   k6 run loadtest/meet-weekend.js
//   BASE_URL=https://api.meetcal.app MEET="2026 Ohio WSO Championships" \
//     HISTORY_CUTOFF=2024-06-13 PEAK_VUS=100 k6 run loadtest/meet-weekend.js
//
// Point BASE_URL at a STAGING copy, not production. Defaults hit a local server.

import http from 'k6/http';
import { check, sleep, group } from 'k6';
import { Trend, Rate } from 'k6/metrics';

const BASE = __ENV.BASE_URL || 'http://127.0.0.1:3000';
const MEET = __ENV.MEET || '2026 USA Weightlifting National Championships, Powered by Rogue Fitness';
const HISTORY_CUTOFF = __ENV.HISTORY_CUTOFF || '2024-06-13';
const PEAK_VUS = Number(__ENV.PEAK_VUS || 100);
// Fraction of users that trigger the heavy offline package download.
const PACKAGE_RATE = Number(__ENV.PACKAGE_RATE || 0.2);

const packageTrend = new Trend('package_duration', true);
const flowErrors = new Rate('flow_errors');

export const options = {
  scenarios: {
    meet_weekend_spike: {
      executor: 'ramping-vus',
      startVUs: 0,
      stages: [
        { duration: '30s', target: Math.ceil(PEAK_VUS / 2) }, // doors open
        { duration: '1m', target: PEAK_VUS }, // peak concurrency
        { duration: '2m', target: PEAK_VUS }, // sustain through a session
        { duration: '30s', target: 0 }, // wind down
      ],
      gracefulRampDown: '10s',
    },
  },
  thresholds: {
    // Fail the run if these are breached, so it's CI/gate-friendly.
    http_req_failed: ['rate<0.01'], // <1% HTTP errors overall
    'http_req_duration{kind:light}': ['p(95)<800'], // light endpoints snappy
    package_duration: ['p(95)<5000'], // heavy package under 5s p95
    flow_errors: ['rate<0.01'],
  },
};

function getJson(path, kind) {
  const res = http.get(`${BASE}${path}`, { tags: { kind } });
  const ok = check(res, { [`200 ${path}`]: (r) => r.status === 200 });
  flowErrors.add(!ok);
  return res;
}

export default function () {
  const meet = encodeURIComponent(MEET);

  // 1. App open: health + meet list + details.
  group('open', () => {
    getJson('/health', 'light');
    getJson('/meets', 'light');
    getJson(`/meets/details?meet=${meet}`, 'light');
  });
  sleep(Math.random() * 2);

  // 2. The common path: schedule + start list.
  group('schedule_and_start_list', () => {
    getJson(`/meets/schedule?meet=${meet}`, 'light');
    getJson(`/meets/athletes-sessions?meet=${meet}`, 'light');
  });
  sleep(1 + Math.random() * 3);

  // 3. A minority download the full offline package (the heavy endpoint).
  if (Math.random() < PACKAGE_RATE) {
    group('package', () => {
      const res = http.get(
        `${BASE}/meets/package?meet=${meet}&history_cutoff_date=${HISTORY_CUTOFF}`,
        { tags: { kind: 'package' }, timeout: '30s' },
      );
      packageTrend.add(res.timings.duration);
      const ok = check(res, { 'package 200': (r) => r.status === 200 });
      flowErrors.add(!ok);
    });
  }

  // 4. Browsing comp data / athletes.
  group('browse', () => {
    getJson('/data/wso', 'light');
    getJson(`/meets/athletes?meet=${meet}`, 'light');
  });
  sleep(1 + Math.random() * 4);
}
