---
name: api-performance-benchmark
description: >-
  Benchmark MeetCal Rust API routes against the React Native app client path
  (full query + mapping + cache + filters, not Convex-only). Update
  performance/results.md when adding routes. Clean up one-off scripts after each run.
---

# API performance benchmarking

Compare **Rust `meetcal-backend/app`** HTTP handlers to the **meetcal-app** code path the UI actually runs before adding or changing routes.

## What to measure

| Layer | Measure |
| --- | --- |
| **Rust** | End-to-end `GET` (or write route) via HTTP: Convex call + Rust logic + JSON response. |
| **RN** | Full client path for the same feature: network check (if applicable), Convex query, row mapping, AsyncStorage write (mocked in Node), hooks/filters the screen uses. |

Do **not** record Convex-only time as the RN number unless the route has no extra client work.

## Workflow for a new route

1. Add a row to the table in `performance/results.md` (table only — no prose sections).
2. Implement or confirm the Rust route in `src/routes/`.
3. Add `meetcal-app/scripts/performance/bench-<route>.ts` mirroring the full RN client path for that route.
4. Extend the `case` in `performance/scripts/run-benchmarks.sh` for the route id, path, and RN script.
5. Start Rust server if needed: `cd meetcal-backend/app && cargo run` (`meetcal-backend/.env` → `CONVEX_URL`).
6. Run `performance/scripts/run-benchmarks.sh <route-id>`.
7. Commit updated `results.md`.
8. **Clean up** (see below).

## Scripts (keep only these)

| Script | Purpose |
| --- | --- |
| `performance/scripts/run-benchmarks.sh` | Single entry point: Rust HTTP median + RN bench + updates `results.md` |
| `meetcal-app/scripts/performance/bench-<route>.ts` | One RN bench file per route |

Do **not** add separate Rust curl scripts, one-off runners, or duplicate orchestrators. Rust timing lives inside `run-benchmarks.sh`.

## Clean up after each benchmark run

- **Delete** any temporary bench scripts you created only to debug a single run (scratch files, copied `bench-*.ts`, extra shell wrappers).
- **Do not** leave standalone `bench-rust.sh`-style helpers; use `run-benchmarks.sh` only.
- **Keep** `bench-<route>.ts` files that are registered in `run-benchmarks.sh` and have a row in `results.md`.
- **Remove** `bench-<route>.ts` when the route is removed or renamed (and drop the `case` + table row).
- **Do not** commit server logs, `.env` copies, or raw curl output under `performance/`.

## Environment

- Rust: `meetcal-backend/.env` → `CONVEX_URL`
- RN bench: same URL via `CONVEX_URL` or `EXPO_PUBLIC_CONVEX_URL`
- Default: 10 iterations, 1 warmup; report **median** ms

## Table columns

| Column | Meaning |
| --- | --- |
| **Rust (ms)** | Median HTTP latency for the Rust route |
| **RN client path (ms)** | Median full in-app path (not Convex-only) |
| **Rust vs RN %** | `((rust_ms - rn_ms) / rn_ms) * 100` — negative = Rust faster |
| **User noticeable** | Whether the delta is likely perceptible in the app (see below) |

## User noticeable column

Set automatically by `run-benchmarks.sh`:

- **No** — absolute delta `< 100 ms` **and** `|rust vs rn %| < 25`
- **Yes** — otherwise (≥100 ms slower/faster, or ≥25% relative change)

Tune thresholds here if product expectations change; keep `results.md` rows in sync after re-running benchmarks.
