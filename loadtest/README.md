# Load test

Read-only test of the meet-weekend traffic spike. **Run it against a staging
copy of the API, never production**: it drives ~100 concurrent users through
the heavy `/meets/package` path and the batched `POST /lifting-results/*`
calls the shipped app makes, and would compete with real users for the same
Postgres.

## What it simulates

The request mix of the current app (`X-MeetCal-App: 6.2.0` on every call, so
the strict validation path is exercised):

| Scenario | Share of `PEAK_VUS` | Requests |
|---|---|---|
| `meet_weekend_spike` | 1 − `PACKAGE_RATE` | app open (`/meets`, `/meets/details`, `/meets/schedule`, `/meets/athletes-sessions`), start list (`POST /lifting-results/bests`), attempt estimator (`POST /lifting-results/recent`), one `/clubs/meet-stats`, one `/search?query`, browse |
| `offline_package` | `PACKAGE_RATE` (default 0.2) | `GET /meets/package?meet&history_cutoff_date=<today−2y>`, then `POST /lifting-results/by-names` in batches of ≤ 40 for the whole roster, then three `If-None-Match` revalidates that must return `304` |
| `saved_sessions` | `PEAK_VUS / 10`, only when `CLERK_JWT` is set | `GET /users/me/saved-sessions` |

Names and clubs are seeded once in `setup()` from `/meets/athletes-sessions`,
so any meet with a start list works.

## 1. Pick a staging target

```sh
export BASE_URL=https://staging-api.example.test   # a staging copy, not production
export MEET="2026 Ohio WSO Championships"          # a meet with a start list on that copy
```

Optional: `PEAK_VUS` (default 100), `PACKAGE_RATE` (default 0.2),
`HISTORY_CUTOFF` (default today − 2 years), `APP_VERSION` (default 6.2.0),
`CLERK_JWT` (enables the signed-in scenario; the token must be for the
staging Clerk instance).

## 2. Watch the box (second SSH pane on the staging host — one command)

```sh
watch -n2 'docker stats --no-stream meetcal-api meetcal; echo; \
  docker exec meetcal psql -U postgres -d meetcal \
  -c "SELECT count(*), state FROM pg_stat_activity GROUP BY state;"'
```

## 3. Run k6

From a laptop with k6 installed:

```sh
BASE_URL="$BASE_URL" MEET="$MEET" PEAK_VUS=100 k6 run loadtest/meet-weekend.js
```

Or via Docker, from any host that can reach staging:

```sh
docker run --rm -i \
  -e BASE_URL="$BASE_URL" -e MEET="$MEET" \
  -e PEAK_VUS=100 -e PACKAGE_RATE=0.2 \
  -v "$PWD/loadtest/meet-weekend.js:/script.js" grafana/k6 run /script.js
```

Local smoke test against `cargo run` (defaults to `http://127.0.0.1:3000`):

```sh
PEAK_VUS=10 k6 run loadtest/meet-weekend.js
```

## 4. Pass/fail (from the k6 summary)

The script encodes these as k6 thresholds, so a breach fails the run.

| Metric | Want |
|---|---|
| `http_req_failed` | < 1% |
| `flow_errors` | < 1% |
| `http_req_duration{kind:light}` p95 | < 800 ms |
| `http_req_duration{kind:bests}` / `{kind:recent}` / `{kind:club_stats}` / `{kind:search}` p95 | < 1 s |
| `package_duration` p95 | < 5 s |
| `by_names_duration` p95 (one ≤ 40-name batch) | < 1.5 s |
| `http_req_duration{kind:revalidate}` p95 | < 300 ms |
| `package_revalidate_304` | > 99% |
