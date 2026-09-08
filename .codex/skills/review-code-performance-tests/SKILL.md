---
name: review-code-performance-tests
description: Three ordered passes over MeetCal backend production surfaces — code, performance, then tests. Use after substantive changes or when asked to harden reliability. Open or update a PR; merge only when explicitly authorized and CI is green.
---

# Review: code, performance, tests

Run three passes in order against production surfaces that ship in this repo:

`app/src/`, `scrapers/common/`, `scrapers/usaw/meet_automation/`

Out of scope unless the task names them: `scrapers/urlwatch/`, vendored `sport80_api` copies, generated `app/target/`, and any Convex dual-write. Postgres is the only ingest path. `convex_id` is a Postgres identity column.

Package manager for JS helpers is bun. Rust uses cargo. Python tests use unittest.

Do not push to `master`. Open one PR against `master` with evidence. Merge only when the user explicitly authorizes it and CI is green.

## Pass 1 — Code check

Hunt for:

- Dead code and unused exports on the runtime path
- Unbounded loops / unbounded name-list query params
- Hidden policy in route comments that belongs in a validator
- Missing validation at HTTP, Slack, and ingest boundaries (JWT, Slack HMAC, empty meet/club/wso, empty delete/replace keys)
- Duplicated replace-vs-upsert or name-normalization policy
- Control flow that cannot fail closed (missing `fetch_one` row treated as 500, empty body treated as success, empty `groups` prune wiping intl rankings)
- Reintroduction of Convex clients or `convex_compat`

Fix what is bounded. Add regression tests next to the change. Defer the rest in the PR body.

NASA Power of Ten + TigerStyle from `AGENTS.md` apply.

## Pass 2 — Performance check

Evidence-based only. Do not invent Convex insights, query planners, or mobile-app coverage.

Measure or trace:

- `/meets/package` cache (TTL, entry cap, byte cap) vs rebuild cost
- Name-list endpoints (`/lifting-results/by-names`, `/recent`, `/bests`) — cap and index use
- Club/WSO history queries (`WHERE club = $1` / `WHERE wso = $1`) vs accidental status joins
- Ingest replace: one transaction for delete+insert; no per-row `IngestClient.action` commit in meet automation
- Loadtest notes in `loadtest/` for meet-weekend read traffic

Use existing indexes (`meet`, `club`, `wso`, normalized name). Change only with a before/after story.

## Pass 3 — Test check

1. Run the inventory script: `bun .codex/skills/review-code-performance-tests/scripts/report-coverage-gaps.ts`.
2. If `coverage/lcov.info` exists (from `cargo llvm-cov` or `coverage lcov`), the script will annotate hit rates. Inventory mode works without it.
3. Add **risk-based** tests, not percentage padding:

   - Auth boundaries (missing/empty/forged/expired token, wrong `azp`)
   - Slack HMAC (bad signature, stale timestamp)
   - Empty and max collections (0 names, `MAX_NAME_LIST_LEN + 1` names, empty club/wso)
   - Ingest dispatch unknown path, empty meet delete, empty intl prune noop, replace group identity required
   - Meet automation approval → single-transaction write (rollback if upsert fails after delete)
   - Missing meet / `sqlx::Error::RowNotFound` → 404
   - Error propagation (401 vs 400 vs 404 vs 500)

Do not add tests that only snapshot JSON to move coverage.

If bun is missing, install it (`curl -fsSL https://bun.sh/install | bash`) or run the script with bun from mise. Skip `/home/maddisen/.codex/...` skill-validate paths; they are not in this environment.

## Verify / Deliver

1. `cd app && cargo fmt --all -- --check`
2. `cd app && cargo clippy --all-targets --locked -- -D warnings`
3. `cd app && cargo test --locked`
4. `cd scrapers && PYTHONPATH=. python -m unittest discover -s usaw/meet_automation/tests -p 'test_*.py'`
5. `cd scrapers && PYTHONPATH=. python -m unittest discover -s common/tests -p 'test_*.py' && PYTHONPATH=. python -m unittest common.test_postgres_writer`
6. Open **one** PR against `master`. Do **not** push to `master`. Merge only if explicitly authorized and CI is green.
7. PR body lists: passes run, fixes landed, new tests, deferred gaps, command evidence.

If a pass finds the tree already excellent, say so with evidence (commands + why remaining gaps are deferred).
