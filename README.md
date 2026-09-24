# MeetCal Backend

Backend services for [MeetCal](https://meetcal.app) — a weightlifting meet companion app. This repo holds the Rust API, PostgreSQL schema, and scraper ingestion layer that power meet schedules, start lists, records, standards, and more.

## Performance

This rewrite moved read-path data access from a remote document database to a local PostgreSQL database queried with [SQLx](https://github.com/launchbadge/sqlx). Benchmarks against the previous stack show roughly **25× lower latency** on typical routes — sub-millisecond database time versus ~25–30 ms end-to-end before.

## Repository layout

| Path                                 | Purpose                                                            |
| ------------------------------------ | ------------------------------------------------------------------ |
| [`app/`](app/)                       | Mobile app HTTP API (Axum + SQLx + PostgreSQL)                     |
| [`app/migrations/`](app/migrations/) | SQLx migrations — schema, indexes, and row-level security policies |
| [`scrapers/`](scrapers/)             | Internal HTTP API for scraper pipelines (ingestion)                |

## Stack

- **Rust** — Axum web server, Tokio async runtime
- **PostgreSQL** — primary data store with RLS for read-only API access
- **SQLx** — compile-time checked queries, connection pooling, migrations

## Prerequisites

- Rust (stable)
- [sqlx-cli](https://github.com/launchbadge/sqlx): `cargo install sqlx-cli --no-default-features --features postgres`
- Docker (optional — used by the database init script)

## Getting started

### 1. Environment

Copy the example env file at the repo root and fill in values:

```bash
cp .env.example .env
```

Required variables:

| Variable                 | Description                                      |
| ------------------------ | ------------------------------------------------ |
| `APP_DATABASE__USERNAME` | Postgres role for the API (`postgres` locally; production must use `meetcal_api`) |
| `APP_DATABASE__PASSWORD` | Postgres password for the API                    |
| `RUST_LOG`               | Log filter for `tracing` (default `info`, one line per request) |
| `POSTGRES_PASSWORD`      | Same password, used by `init_db.sh`              |
| `DATABASE_URL`           | Full connection string (URL-encode the password) |
| `CLERK_JWKS_URL`         | Clerk JWKS endpoint used to verify session JWTs  |
| `CLERK_ISSUER`           | Expected Clerk token issuer                      |
| `CLERK_AUTHORIZED_PARTIES` | Comma-separated trusted web origins (`azp`)    |

Optional:

| Variable         | Description                                |
| ---------------- | ------------------------------------------ |
| `SCRAPER_SECRET` | Shared secret for scraper ingestion routes |

Configuration is layered: defaults in [`app/src/configuration.yaml`](app/src/configuration.yaml), optional overrides in `app/src/configuration.local.yaml`, and env vars prefixed with `APP_` (e.g. `APP_DATABASE__PASSWORD`).

### 2. Database

Start Postgres and apply migrations:

```bash
cd app/scripts
./init_db.sh
```

This creates a Docker container named `meetcal` (Postgres 16, `--encoding=UTF8 --locale=C.UTF-8`), creates the `meetcal` database, and runs all SQLx migrations. Set `SKIP_DOCKER=1` if you already have Postgres running locally. The database must be UTF-8 with a ctype that folds Unicode (`C.UTF-8`, `en_US.UTF-8`, or ICU), never `C`/`POSIX`: the script and the API both refuse otherwise, because Postgres-side name matching (`lower()`, `\s`) would then miss every non-ASCII name. `LC_CTYPE` cannot be changed after `CREATE DATABASE`.

### 3. Run the API

```bash
cd app
cargo run --release
```

The server listens on `http://127.0.0.1:3000` by default.

## App API routes

| Method | Path                 | Description                       |
| ------ | -------------------- | --------------------------------- |
| `GET`  | `/meets`             | Upcoming meets (next 3 months)    |
| `GET`  | `/meet-details`      | Single meet metadata              |
| `GET`  | `/meets/package`     | Selected meet data package (`ETag` / `If-None-Match` → `304`); optional `include=year_bests,recent_results,attempt_estimates` (absent = all). `/meets`, `/meets/details`, `/meets/schedule` and the reference-data routes also send `ETag` + `Cache-Control: no-cache` (revalidate with `If-None-Match`). Timeouts answer `408 {"error":"timeout"}`; `/health` returns `{"status":"ok","db":{"size","idle"}}` |
| `GET`  | `/meets/schedule`    | Session schedule for a meet       |
| `GET`  | `/meets/athletes`    | Start list with session timing    |
| `GET`  | `/clubs`             | Club directory                    |
| `GET`  | `/records`           | National/world records            |
| `GET`  | `/wso`               | Weightlifting state organizations |
| `GET`  | `/wso-records`       | State-level records               |
| `GET`  | `/standards`         | Competition standards             |
| `GET`  | `/qualifying-totals` | Qualifying totals                 |
| `GET`  | `/intl-rankings`     | International rankings            |
| `GET`  | `/nat-rankings`      | National rankings                 |
| `GET`  | `/adaptive`          | Adaptive division records (optional `season=YYYY`) |
| `GET`  | `/search`            | Result search                     |
| `GET` `POST` | `/lifting-results/by-names` | Full history for a list of names (`POST {"names": [...]}`); optional `latest_only=true` or `limit_per_name=1..200` |
| `GET` `POST` | `/lifting-results/recent` | History since `cutoff_date` for a list of names |
| `GET`  | `/lifting-results/year` | Best lifts for one name since `cutoff_date` |
| `GET` `POST` | `/lifting-results/bests` | Best lifts since `cutoff_date`, keyed by requested name |
| `GET`  | `/users/me/saved-sessions` | Saved sessions for authenticated user |
| `PUT`  | `/users/me/saved-sessions/{session_id}` | Upsert saved session |
| `DELETE` | `/users/me/saved-sessions/{session_id}` | Delete saved session |
| `DELETE` | `/users/me/saved-sessions` | Clear saved sessions |
| `GET`  | `/users/me/preferences` | Preferences for authenticated user |
| `PATCH` | `/users/me/preferences/auto-unsave` | Toggle auto-unsave preference |
| `POST` | `/scrapers/slack/commands` | Slack slash commands to manage scraper lists |
| `POST` | `/scrapers/slack/interactions` | Slack Approve/Reject buttons for staged meet uploads |

Responses are gzip- and Brotli-compressed.

The mobile app sends `X-MeetCal-App: <major.minor.patch>`. Clients at or above the version in `app/src/common/client.rs` opt into fail-closed validation (`400` on a blank `wso`, a missing or malformed `cutoff_date`, or malformed search dates); older or absent headers get the legacy behaviour, so shipped builds keep working across a rollout.

### Rate limits

The API is open: no key is needed. To keep it fast for everyone on meet day, each client spends tokens from a bucket that refills continuously:

| Client | Refill | Burst |
| --- | --- | --- |
| Anonymous, per IPv4 address or IPv6 `/64` | 40 tokens/s | 1,200 tokens |
| With an API key (`X-MeetCal-Key`) | 200 tokens/s | 6,000 tokens |

Most requests cost 1 token. `/meets/package` costs 4; `/search`, `/lifting-results/by-names`, `/lifting-results/recent`, `/lifting-results/bests` (`GET` or `POST`) and `/clubs/meet-stats` cost 5. `/health` is free.

- **`429 {"error":"rate limited"}`** means your bucket is empty. The `Retry-After` header says, in whole seconds, when it will hold enough tokens for that request again. Wait at least that long before retrying.
- **`503 {"error":"overloaded"}`** with `Retry-After: 1` means the server as a whole is at capacity, whoever is asking. Retry after a second, with a cap on attempts.
- Both are JSON, and browsers can read them cross-origin: `Retry-After` is listed in `Access-Control-Expose-Headers`.
- Spend fewer tokens by sending up to 100 names per name-list request (`POST {"names": [...]}`) instead of one request per athlete, and by caching what you fetch.

Limits are being introduced in log-only mode first, so you may not see a `429` yet. Build for one anyway.

**API keys** are for server-side integrations that need more than the anonymous budget. Send the secret as `X-MeetCal-Key: <secret>`. Never put a key in a browser or mobile app, where anyone can read it; browsers cannot send the header cross-origin anyway. An unknown key is treated exactly like no key: no error, the anonymous budget. To request a key, open an issue on this repository with your project, a contact, and your expected request volume. The secret is then shared privately.

Operators: settings, production env vars and the shadow-mode rollout are in [`docs/rate-limits.md`](docs/rate-limits.md).

### Slack control surfaces (scraper lists + approvals)

The API exposes two Slack endpoints — its only mutating surfaces. They edit JSON
files on the server's disk and drop approval decisions; they touch no database.
Because those files live next to the cron jobs that read them, changes take
effect on the running server with **no redeploy or git pull**. Both are disabled
(HTTP 503) until `SLACK_SIGNING_SECRET` is set, and every request is
signature-verified (optionally restricted to a user allowlist).

**`POST /scrapers/slack/commands`** — `list` / `add` / `delete`, routed by the
**command name** (so one Slack channel can host both lists, or you can split
them across channels):

| Command group | Edits | Forms |
| --- | --- | --- |
| `/meet-*` | [`watches.json`](scrapers/usaw/meet_automation/watches.json) | `add <key> \| <meet name> \| <page url> [\| <start-list url> \| <schedule url>]`, `delete <key>`, `list`, `run <key>` |
| `/entries-*` | [`entries_targets.json`](scrapers/usaw/entry_scraper/entries_targets.example.json) | `add <label> \| <entries url>`, `delete <label>`, `list` |

Channels (`SLACK_MEET_AUTOMATION_CHANNEL`, `SLACK_ENTRIES_CHANNEL`) act as an
optional allowlist. The entries job (`run_scraper_job.sh entries`) reads
`entries_targets.json` each run, falling back to a built-in list when absent.

`/meet-run <key>` (or `/meet-run all`) triggers the pipeline on demand: it drops
a request under `MEET_AUTOMATION_STATE_DIR/run_requests/`, which the pipeline's
`run --requested` cron drains within a couple minutes and posts the usual Slack
review. The API never runs the scrape itself — same file-handshake reasoning as
the buttons below.

**`POST /scrapers/slack/interactions`** — receives the *Approve & publish* /
*Reject* buttons the meet-automation pipeline posts. A click records a decision
under `MEET_AUTOMATION_STATE_DIR/decisions/`, which the pipeline's `approve`
cron consumes to write Postgres in a single transaction. The DB write stays
in the Python pipeline, so this API keeps no database credentials.

Full server setup — Slack app config, env vars, cron, preview hosting, testing —
is in [`docs/meet-automation-setup.md`](docs/meet-automation-setup.md). See also
the Slack-related variables in [`.env.example`](.env.example).

## Development

```bash
cd app

# Lint
cargo clippy --all-targets -- -D warnings

# Prepare local integration-test data.
# This truncates app tables in DATABASE_URL, so the reset flag is required.
MEETCAL_ALLOW_TEST_DB_RESET=1 scripts/setup_test_db.sh

# Tests run against a locally spawned API server.
cargo test

# Apply new migrations after editing app/migrations/
sqlx migrate run
```

**Production order:** run `sqlx migrate run` (as the table owner, not `meetcal_api`) *before* merging a change that adds migrations. Merging to master deploys automatically, and the API refuses to start against a database missing any migration it was built with; `app/deploy/deploy-prod.sh` then waits for `/health`, and if the new container never answers it restores the previous one and fails the workflow. Never edit a migration file after it has been applied anywhere: sqlx records its checksum and `migrate run` refuses a changed file.

CI starts Postgres, runs migrations, loads [`app/scripts/seed_test_db.sql`](app/scripts/seed_test_db.sql), then runs clippy and `cargo test` on every push via [`.github/workflows/ci.yml`](.github/workflows/ci.yml).
