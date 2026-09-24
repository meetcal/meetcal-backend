# MeetCal Backend

Rust + PostgreSQL API and scraper ingest for USA Weightlifting meet schedules, start lists, records, standards, and companion data. Postgres is the only ingest path. `convex_id` is a stable Postgres identity column, not a Convex client.

Sister repos (do not implement them here): `meetcal-app` (Expo / React Native), `meetcal-web`, `meetcal-cli`.

## Layout

| Path | Role |
|---|---|
| `app/` | Axum HTTP API (read path + Slack control surfaces) |
| `app/src/routes/` | Route handlers. Keep SQL and validation in the handler or a nearby helper; do not hide policy in comments |
| `app/src/routes/users/` | Clerk JWT auth, saved sessions, preferences. Fail closed when Clerk is unset |
| `app/src/routes/scrapers/` | Slack slash commands + Approve/Reject. File handshake only; the one DB write is the venue-map `UPDATE` on `meets.venue_map_pdf_url` / `venue_map_apple_url` (the sole `UPDATE` granted to `meetcal_api`) |
| `app/migrations/` | SQLx migrations, indexes, RLS |
| `app/tests/` | HTTP integration tests against a spawned server + seed DB |
| `scrapers/common/` | Postgres writer + ingest dispatch (`postgres_writer.py`, `postgres_ingest.py`) |
| `scrapers/usaw/meet_automation/` | Watch → scrape → validate → Slack approve → single-transaction Postgres write |
| `scrapers/usaw/` `scrapers/iwf/` `scrapers/usamw/` `scrapers/bwl/` | Federation scrapers. They call ingest dispatch; they must not talk to Convex |
| `docs/` | Ops runbooks (`meet-automation-setup.md`, `rate-limits.md`) and `testing.md` |
| `.codex/skills/` | Agent skills, including the three-pass review skill |
| `loadtest/` | k6 meet-weekend scenarios |

## Commands

Package manager for JS helpers is **bun**. Do not use npm. Rust uses cargo. Python scraper tests use the stdlib unittest runner.

| Task | Command |
|---|---|
| Install JS helper | `bun install` (only if a lockfile exists for a helper script) |
| API dev server | `cd app && cargo run` |
| Init local Postgres | `cd app/scripts && ./init_db.sh` (`SKIP_DOCKER=1` if Postgres is already up) |
| Seed test DB | `MEETCAL_ALLOW_TEST_DB_RESET=1 app/scripts/setup_test_db.sh` |
| Format | `cd app && cargo fmt --all` |
| Lint | `cd app && cargo clippy --all-targets --locked -- -D warnings` |
| Rust tests | `cd app && cargo test --locked` |
| Python compile | `cd scrapers && python -m compileall -q common iwf usaw usamw bwl` |
| Meet automation tests | `cd scrapers && PYTHONPATH=. python -m unittest discover -s usaw/meet_automation/tests -p 'test_*.py'` |
| Postgres ingest tests | `cd scrapers && PYTHONPATH=. python -m unittest discover -s common/tests -p 'test_*.py'` (includes `test_normalize`, `test_dedupe_idless_athletes`, `test_complete_ended_meets`) |
| WSO writer tests | `cd scrapers && PYTHONPATH=. python -m unittest common.test_postgres_writer` |
| Coverage gaps | `bun .codex/skills/review-code-performance-tests/scripts/report-coverage-gaps.ts` |
| Shellcheck | `shellcheck -x -P app/scripts app/deploy/*.sh app/scripts/*.sh` |

`DATABASE_URL` is required for writer/ingest tests. They skip when it is unset so meet-automation unit tests still run without Postgres.

## Verify

Run this table before opening or updating a PR. Do not push to `master`. Merge only when the user explicitly authorizes it and CI is green.

| Gate | Command | Pass when |
|---|---|---|
| Format | `cd app && cargo fmt --all -- --check` | Exit 0 |
| Clippy | `cd app && cargo clippy --all-targets --locked -- -D warnings` | Exit 0 |
| Rust tests | `cd app && cargo test --locked` | Exit 0 |
| Meet automation | `cd scrapers && PYTHONPATH=. python -m unittest discover -s usaw/meet_automation/tests -p 'test_*.py'` | Exit 0 |
| Ingest / writer | `cd scrapers && PYTHONPATH=. python -m unittest discover -s common/tests -p 'test_*.py' && PYTHONPATH=. python -m unittest common.test_postgres_writer` | Exit 0 |
| Coverage inventory | `bun .codex/skills/review-code-performance-tests/scripts/report-coverage-gaps.ts` | Report written; no new untested auth, Slack, dispatch, writer, or prune holes in files you touched |

CI (`.github/workflows/ci.yml`) runs the Rust job (fmt, clippy, `cargo test --locked`, shellcheck, docker build), the Python job (compileall + the three unittest invocations), and `cargo audit`.

## Code Quality

- Validate untrusted input at the HTTP and ingest boundaries before SQL or filesystem writes.
- Bound name-list query params. Empty and oversized lists fail closed.
- Slack mutating surfaces verify HMAC signatures and write files, not Postgres, except `/meets-add-*` / `/meets-remove-*`, which `UPDATE` only the two venue-map columns on `meets`. Approval → ingest is a Python single transaction.
- Blank required params (`meet`, `club`, `wso`, `name`, `query`, `year`) fail with `400` only for strict clients via `ClientVersion`; legacy clients keep the pre-2026-09-08 answers (list endpoints short-circuit to `[]` rather than query `= ''`). Name lists stay always-strict. The `/meets/package` cache is invalidated by a per-meet freshness stamp (row counts + max `xmin` of the meet's rows and the meet row), with the TTL as a backstop for history from other meets.
- Changing what an existing parameter means (a date range's end, a default window) is a contract change like a status code: keep what shipped apps send working. `/search` ranges are half-open because every app sends `YYYY+1-01-01` as the end.
- Migrations: apply before merging (deploys are automatic; the API exits when the database is behind it, and when the database ctype is `C`/`POSIX` or the encoding is not UTF-8, since Postgres-side `lower()` / `\s` would then disagree with `normalize_name` on non-ASCII names). Never edit an applied migration file. Reference-data responses send `Cache-Control: no-cache` with a strong `ETag`, never a `max-age` that shipped apps cannot bypass.
- Ingest loops go through `IngestClient.actions(path, rows)` (one connection, one transaction), never per-row `action()` in a loop. JS scrapers likewise send a run's rows to one `postgres_ingest.py` process (`--skip-errors` when a bad row should be skipped, not sink the batch), chunked under the stdin caps, never one process per row. The Python DB tests apply `app/migrations` themselves, so a fresh CI database works.
- `convex_id` is the upsert identity in Postgres. Do not add a Convex client, dual-write, or `convex_compat`.
- Destructive ingest (`DELETE FROM … WHERE meet = $1`, intl ranking prune) must refuse empty keys.
- Prefer indexes (`meet`, `club`, `wso`, normalized name) over `filter()`-style scans. Name match uses `normalize_name` / `normalized_name_sql!` (`app/src/common/names.rs`), the one spelling of the rule; `concat!` it into a query rather than retyping it.
- Shipped app builds cannot be patched in lockstep with the API. Stricter validation is gated on the `X-MeetCal-App: <major.minor.patch>` header via `ClientVersion` (`app/src/common/client.rs`): at or above `MIN_STRICT_CLIENT_VERSION` a request fails closed with `400`; older or absent means the legacy behaviour it was built against. Raise the threshold when a new app depends on a stricter contract; remove a legacy branch only once the version tail that needs it is gone.
- Name-list endpoints (`/lifting-results/by-names`, `/recent`, `/bests`) accept `POST {"names": [...], "cutoff_date"?}` alongside the CSV `GET`. The JSON array is the only form that can carry a name containing a comma.
- Auth is Clerk JWT (RS256 + JWKS). Protected `/users/me/*` routes fail closed when Clerk env is missing or the token is empty, expired, or wrong `azp`.
- Club and WSO history endpoints return every registration for that affiliation, including non-completed meets. Do not re-join `meets.status`.

## Reliability Guideposts

### NASA Power of Ten (adapted)

1. **Simple control flow.** Slack command parsing and ingest dispatch are explicit matches, not hidden state machines.
2. **Bounded loops.** Cap name lists, package cache entries, and Slack run ids. Never `DELETE`/`replace` a whole table from an empty payload.
3. **No dynamic allocation surprises on the hot path.** `/meets/package` has entry, byte, and TTL caps.
4. **Declare sizes.** Timeouts (API 15s, Slack HMAC 5-minute skew, JWKS refresh 60s), `MAX_NAME_LIST_LEN`, `MAX_NAME_LEN` (400 bytes), package cache constants and `MAX_CONCURRENT_PACKAGE_BUILDS`. Request bodies: `DEFAULT_BODY_LIMIT` (1 MiB), `NAME_LIST_BODY_LIMIT` (256 KiB) and `USER_WRITE_BODY_LIMIT` (160 KiB), each with a compile-time check that a maximal valid body fits even with every character JSON-escaped; over-limit is a JSON `413`. Ingest stdin: `MAX_STDIN_BYTES` (16 MiB), `MAX_STDIN_ROWS` (20,000). Abuse limits (`app/src/common/rate_limit.rs`, `load_shed.rs`): per-client token buckets (anonymous `DEFAULT_IP_TOKENS_PER_SECOND` 40/s, `DEFAULT_IP_BURST` 1,200 per IPv4 address or IPv6 `/64`; keyed `DEFAULT_KEY_TOKENS_PER_SECOND` 200/s, `DEFAULT_KEY_BURST` 6,000), route costs (`*_ROUTE_COST`: 1 by default, 4 package but only `PACKAGE_REVALIDATE_COST` 1 for a `304` revalidation, 5 search / name lists / meet-stats), `MAX_API_KEYS` (32), `MAX_TRACKED_CLIENTS` (1M) swept every `CLIENT_EVICTION_INTERVAL` (60s), and `DEFAULT_MAX_IN_FLIGHT` (500 = `MAX_DB_CONNECTIONS` x `DB_ACQUIRE_TIMEOUT` / `SLOW_REQUEST_DB_TIME`). Compile-time checks pin the defaults to a 500-phone venue behind one NAT (background sync, and every phone's `HISTORY_REFRESH_COST` within the first hour) and a 40-request meetcal-cli burst; the first-minute gap is documented in `docs/rate-limits.md`, which says to raise the anonymous burst before enforcing. Over the limit is a JSON `429` with `Retry-After` (enforced only with `APP_RATE_LIMIT__ENFORCE=true`; shadow mode logs one redacted warning per client per minute); over the in-flight cap is always a JSON `503` with `Retry-After: 1`. `/health` is exempt from both, and CORS wraps both so browsers can read them. API keys come only from `APP_RATE_LIMIT__KEYS`, stored as SHA-256 digests; a bad key is anonymous, never a `401`.
5. **Check returns.** `fetch_one` missing rows are 404, not 500. Slack signature failure is 401.
6. **Data hiding at boundaries.** Scraper payloads are cleaned of `scraperSecret` before write. JWT `sub` is the only user id.
7. **Check return values of calls.** HMAC, `create_dir_all`, decision rename, and `conn.commit()` are not fire-and-forget on the write path.
8. **Limit preprocessor / magic.** One `DATABASE_URL`, one ingest dispatch table, one Slack signing secret.
9. **Limit aliases.** Meet name, club, and WSO strings are exact. Do not keep a second copy of replace-vs-upsert policy in a scraper.
10. **Compile with warnings; test the edges.** Clippy `-D warnings`, empty/oversized inputs, rollback after failed replace.

### TigerStyle (adapted)

- **Problems, not solutions.** Measure a slow package rebuild or a wipe of intl rankings before rewriting a scraper.
- **Assertions.** Empty meet on delete, empty WSO replace, empty intl prune, invalid run id — fail loudly.
- **Explicit vs implicit.** Approval is a file in `state/decisions/`; the cron is what writes Postgres.
- **Zero, one, many, max.** Empty name lists short-circuit. Huge name lists reject. Package cache evicts.
- **Design for failure.** Replace athletes+schedule in one transaction so a failed upsert does not leave a meet empty.
- **Show your work.** Scraper Slack notifications and `[perf]` loadtest notes belong in evidence, not production request logs.

## Recurring lessons

- Postgres is the only ingest path. `convex_id` stays as the document identity in SQL. Do not reintroduce Convex.
- Meet automation: Slack Approve/Reject drops a decision file. The `approve` cron loads the staged bundle and writes athletes + schedule in **one** transaction (`replace` deletes then inserts, then `commit`). The Rust API must not gain DB credentials for that write.
- `deleteMissingIntlRankingGroups` with an empty `groups` list is a noop on purpose. Never prune when the scrape produced zero groups.
- `replaceIntlRankingsForGroup` / `replaceWSORecordSet` are exact-set syncs: delete keys that disappeared, upsert the rest. Do not `DELETE` the whole group then insert if a later row can fail.
- Club/WSO athlete history is registration history, not "current club on completed meets only".
- Name matching is case- and whitespace-insensitive. Callers may send `ALEXANDER  NORDSTROM`; responses keep the requested key on batch bests.
- Slack request timestamps older than five minutes are replays. Run ids must be `[A-Za-z0-9._-]`, never path separators.
- JWT tests may use `rsa` (RUSTSEC-2023-0071 ignored). Production verification uses `jsonwebtoken`'s rust_crypto backend and Clerk's JWKS.
- `scrapers/urlwatch/` is vendored third-party. Out of scope unless the task names it.
- An app release is not a flag day: App Store rollouts are gradual, so the API serves old and new clients at once for weeks. Never flip an endpoint's status code or shape globally; gate it on `X-MeetCal-App`.
- `/meets/package` carries a strong `ETag` (SHA-256 of the exact body) and answers `If-None-Match` with `304`. The app persists the tag only after a fully successful prefetch, and ignores a `304` when its local copy is gone.
