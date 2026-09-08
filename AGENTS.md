# MeetCal Backend

Rust + PostgreSQL API and scraper ingest for USA Weightlifting meet schedules, start lists, records, standards, and companion data. Postgres is the only ingest path. `convex_id` is a stable Postgres identity column, not a Convex client.

Sister repos (do not implement them here): `meetcal-app` (Expo / React Native), `meetcal-web`, `meetcal-cli`.

## Layout

| Path | Role |
|---|---|
| `app/` | Axum HTTP API (read path + Slack control surfaces) |
| `app/src/routes/` | Route handlers. Keep SQL and validation in the handler or a nearby helper; do not hide policy in comments |
| `app/src/routes/users/` | Clerk JWT auth, saved sessions, preferences. Fail closed when Clerk is unset |
| `app/src/routes/scrapers/` | Slack slash commands + Approve/Reject. File handshake only; no DB writes |
| `app/migrations/` | SQLx migrations, indexes, RLS |
| `app/tests/` | HTTP integration tests against a spawned server + seed DB |
| `scrapers/common/` | Postgres writer + ingest dispatch (`postgres_writer.py`, `postgres_ingest.py`) |
| `scrapers/usaw/meet_automation/` | Watch → scrape → validate → Slack approve → single-transaction Postgres write |
| `scrapers/usaw/` `scrapers/iwf/` `scrapers/usamw/` `scrapers/bwl/` | Federation scrapers. They call ingest dispatch; they must not talk to Convex |
| `docs/` | Ops runbooks (`meet-automation-setup.md`) and `testing.md` |
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
| Postgres ingest tests | `cd scrapers && PYTHONPATH=. python -m unittest discover -s common/tests -p 'test_*.py'` |
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
- Slack mutating surfaces verify HMAC signatures and write files, not Postgres. Approval → ingest is a Python single transaction.
- `convex_id` is the upsert identity in Postgres. Do not add a Convex client, dual-write, or `convex_compat`.
- Destructive ingest (`DELETE FROM … WHERE meet = $1`, intl ranking prune) must refuse empty keys.
- Prefer indexes (`meet`, `club`, `wso`, normalized name) over `filter()`-style scans. Name match uses `normalize_name` / `NORMALIZED_NAME_SQL`.
- Auth is Clerk JWT (RS256 + JWKS). Protected `/users/me/*` routes fail closed when Clerk env is missing or the token is empty, expired, or wrong `azp`.
- Club and WSO history endpoints return every registration for that affiliation, including non-completed meets. Do not re-join `meets.status`.

## Reliability Guideposts

### NASA Power of Ten (adapted)

1. **Simple control flow.** Slack command parsing and ingest dispatch are explicit matches, not hidden state machines.
2. **Bounded loops.** Cap name lists, package cache entries, and Slack run ids. Never `DELETE`/`replace` a whole table from an empty payload.
3. **No dynamic allocation surprises on the hot path.** `/meets/package` has entry, byte, and TTL caps.
4. **Declare sizes.** Timeouts (API 15s, Slack HMAC 5-minute skew, JWKS refresh 60s), `MAX_NAME_LIST_LEN`, package cache constants.
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
