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

## Python ingest (`scrapers/`)

```sh
cd scrapers
PYTHONPATH=. python -m compileall -q common iwf usaw usamw bwl
PYTHONPATH=. python -m unittest discover -s usaw/meet_automation/tests -p 'test_*.py'
PYTHONPATH=. python -m unittest discover -s common/tests -p 'test_*.py'
PYTHONPATH=. python -m unittest common.test_postgres_writer
```

`common/tests/test_postgres_ingest.py` needs `DATABASE_URL` and psycopg. It skips when either is missing so meet-automation unit tests still run without Postgres.

Risk cases that belong in Python tests:

- Ingest dispatch unknown path
- Empty `meet` on delete athletes/schedule
- Empty intl ranking group identity
- Empty `groups` prune is a noop
- Exact-set sync (WSO records, intl ranking groups) writes only changes
- Meet automation: empty parse is not staged; approval decision files; ingest refuses empty `meet_name`; replace+insert rolls back if a later write fails

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
