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
| `APP_DATABASE__PASSWORD` | Postgres password for the API                    |
| `POSTGRES_PASSWORD`      | Same password, used by `init_db.sh`              |
| `DATABASE_URL`           | Full connection string (URL-encode the password) |

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

This creates a Docker container named `meetcal` (Postgres 16), creates the `meetcal` database, and runs all SQLx migrations. Set `SKIP_DOCKER=1` if you already have Postgres running locally.

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
| `GET`  | `/adaptive`          | Adaptive division records         |
| `GET`  | `/search`            | Result search                     |

Responses are gzip- and Brotli-compressed.

## Development

```bash
cd app

# Lint
cargo clippy --all-targets -- -D warnings

# Tests (requires a running Postgres instance with migrated schema)
cargo test --all-targets

# Apply new migrations after editing app/migrations/
sqlx migrate run
```

CI runs clippy and tests on every push via [`.github/workflows/ci.yml`](.github/workflows/ci.yml).
