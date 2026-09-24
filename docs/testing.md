# Testing

## Rust API (`app/`)

Integration tests spawn the Axum server against the database in `DATABASE_URL` after `app/scripts/setup_test_db.sh` loads `app/scripts/seed_test_db.sql`. The reset flag is required because setup truncates app tables.

```sh
cp .env.example .env   # fill APP_DATABASE__* and DATABASE_URL
cd app/scripts && SKIP_DOCKER=1 MEETCAL_ALLOW_TEST_DB_RESET=1 ./setup_test_db.sh
cd ../app
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

CI (`.github/workflows/ci.yml`) starts Postgres 16, runs that setup, then fmt, clippy, `cargo test --locked`, shellcheck, and a production container build.

Put HTTP tests in `app/tests/` next to the surface they cover (`users.rs`, `scrapers.rs`, `clubs.rs`, `wsos.rs`). Unit tests for pure helpers live in the same `.rs` file under `#[cfg(test)]`.

Risk cases that belong in Rust tests:

- Clerk JWT (missing, empty, forged, expired, wrong `azp`)
- Slack HMAC (bad signature, stale timestamp, path-unsafe run id)
- Empty / oversized `names` on `/lifting-results/by-names`, `/recent`, `/bests`
- Empty `club` / `wso` on history endpoints
- Missing meet → 404 (`sqlx::Error::RowNotFound`), not 500
- Saved-session validation (empty meet/platform, oversized `athlete_names`)
- Rate limits and load shedding (`tests/rate_limits.rs`): `429` + `Retry-After` past the burst, key vs anonymous budgets, bad key is anonymous, `X-Forwarded-For` trust and rightmost entry, IPv6 `/64` grouping, `/health` exempt, shadow mode, `503` at the in-flight cap, CORS on both. Tests build servers with `spawn_app_with_limits`; every other test runs in shadow mode.

## Python ingest (`scrapers/`)

```sh
cd scrapers
PYTHONPATH=. python -m compileall -q common iwf usaw usamw bwl
PYTHONPATH=. python -m unittest discover -s usaw/meet_automation/tests -p 'test_*.py'
PYTHONPATH=. python -m unittest discover -s common/tests -p 'test_*.py'
PYTHONPATH=. python -m unittest common.test_postgres_writer
```

The DB-backed tests under `common/tests/` (`test_postgres_ingest.py`, `test_dedupe_idless_athletes.py`, `test_complete_ended_meets.py`) need `DATABASE_URL` and psycopg. They skip when either is missing so meet-automation unit tests still run without Postgres. `test_normalize.py`, `test_placeholder_parity.py` (Python vs JS `noid:` placeholder rule on `fixtures/placeholder_member_ids.json`; the JS half runs `node` and skips with a message when it is not on PATH), and `test_ingest_callers.py` (scrapers batch through `IngestClient.actions` / `actions_skipping_errors`, plus a static check that no scraper calls `.action(` inside a loop) have no DB dependency.

The schema for those tests is the real one: `test_postgres_ingest.apply_migrations` applies every `app/migrations/*.sql` in filename order and records each version in `_sqlx_migrations` the way `sqlx migrate run` does, skipping versions already recorded. Point `DATABASE_URL` at an empty database (CI does) or one sqlx already migrated; a migration that drifts from what the writer expects fails the Python job instead of being masked by a hand-written copy of the DDL.

Risk cases that belong in Python tests:

- Ingest dispatch unknown path
- Empty `meet` on delete athletes/schedule
- Empty intl ranking group identity
- Empty `groups` prune is a noop
- Exact-set sync (WSO records, intl ranking groups) writes only changes
- `IngestClient.actions` is one transaction: a failing row rolls back the batch
- `upsert_lifting_result` lookup precedence: `convex_id`, then `legacy_id`, then the natural key
- Id-less athletes (blank or `noid:` member id) update one row across re-ingests; platform casing and `h:mm AM/PM` times are canonicalised at ingest
- `complete-ended-meets` compares `end_date` against the meet-local date and tolerates unknown `time_zone` values
- Meet automation: empty parse is not staged; approval decision files; a failing approved ingest is parked as `failed` and its decision consumed; reply classification ("no issues, ship it" approves); ingest refuses empty `meet_name`; replace+insert rolls back if a later write fails

## Coverage inventory

```sh
bun .codex/skills/review-code-performance-tests/scripts/report-coverage-gaps.ts
```

Optional line coverage:

```sh
cd app && cargo llvm-cov --locked --lcov --output-path ../coverage/lcov.info
cd scrapers && python -m coverage run -m unittest discover -s common/tests -p 'test_*.py'
```

Do not pad coverage with snapshot-only tests. Do not call production `https://api.meetcal.app` from tests.
