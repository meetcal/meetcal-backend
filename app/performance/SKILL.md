---
name: api-performance-benchmark
description: >-
  Benchmark MeetCal Rust API routes vs the full meetcal-app client path. Use when adding or
  changing routes in meetcal-backend/app, running performance/scripts/run-benchmarks.sh,
  updating performance/results.md, or the user mentions API latency, benchmarks, or Rust vs RN
  speed. Run on every new route; Total (avg) row recalculates each run.
---

# API performance benchmarking

Compare **Rust `meetcal-backend/app`** HTTP handlers to the **meetcal-app** code path the UI actually runs.

## Agent: run this on every new route

When you add or materially change a Rust API route, **always** finish by running the benchmark for that route. Do not skip because the delta looks small — the goal is a fast app and a complete `results.md` table.

1. Implement the Rust route and register it in `main.rs`.
2. Add `meetcal-app/scripts/performance/bench-<route>.ts` (full RN client path, not Convex-only).
3. Add a `case` in `performance/scripts/run-benchmarks.sh` (route id, HTTP path, RN script).
4. Run (Rust side always uses a **release** build — the script runs `cargo run --release`):

   ```bash
   cd meetcal-backend/app && bash performance/scripts/run-benchmarks.sh '<route-id>'
   ```

   Re-benchmark every route after a batch of changes:

   ```bash
   cd meetcal-backend/app && bash performance/scripts/run-benchmarks.sh all
   ```

5. Confirm `performance/results.md` has the route row and an updated **`Total (avg)`** row at the bottom.
6. **Clean up** scratch scripts (see below). Commit `results.md` when the user asks for a commit.

Re-run the same command when you change handler logic for an existing route so that row and the total stay current.

**Do not** hand-edit per-route numbers or the total row — `run-benchmarks.sh` writes them. You may add a placeholder route row before the first run; the script replaces it and refreshes the total.

## What to measure

| Layer | Measure |
| --- | --- |
| **Rust** | End-to-end `GET` (or write route) via HTTP: Convex call + Rust logic + JSON response. |
| **RN** | Same processing as the Rust handler for that route (Convex query + filter/sort/map the API does). **Not** Convex-only. **Not** local UI filter/sort changes after load. |

Each `bench-<route>.ts` must mirror the matching `src/routes/` handler logic step-for-step (read handlers; do not edit them).

## Workflow for a new route

1. Implement or confirm the Rust route in `src/routes/`.
2. Add `meetcal-app/scripts/performance/bench-<route>.ts` mirroring the full RN client path.
3. Extend the `case` in `performance/scripts/run-benchmarks.sh` for the route id, path, and RN script.
4. **Run** `performance/scripts/run-benchmarks.sh <route-id>` (or `all`) — required on every new or changed route. The script kills port 3000, starts `cargo run --release`, runs the bench, then stops the server (`meetcal-backend/.env` → `CONVEX_URL`).
5. Set `BENCH_MANAGE_SERVER=0` only if you already have a release server on port 3000 and do not want the script to restart it.
6. Verify `performance/results.md`: new route row + recalculated **`Total (avg)`**.
7. Commit updated `results.md` when requested.
8. **Clean up** (see below).

### Route ids in use

| Route id | HTTP | RN bench |
| --- | --- | --- |
| `meets` | `GET /meets` | `bench-meets.ts` |
| `meets/:name` | `GET /meets/{encoded-name}` | `bench-meet-details.ts` |
| `meets/schedule/:name` | `GET /meets/schedule/{encoded-name}` | `bench-meet-schedule.ts` |
| `meets/athletes/:name` | `GET /meets/athletes/{encoded-name}` | `bench-meet-athletes.ts` |

Use `BENCH_MEET_NAME` (`meets/:name`), `BENCH_SCHEDULE_MEET_NAME`, or `BENCH_ATHLETES_MEET_NAME` when defaults are wrong.

## Scripts (keep only these)

| Script | Purpose |
| --- | --- |
| `performance/scripts/run-benchmarks.sh` | Single entry point: Rust HTTP median + RN bench + updates `results.md` + **Total (avg)** |
| `meetcal-app/scripts/performance/bench-<route>.ts` | One RN bench file per route |

Do **not** add separate Rust curl scripts, one-off runners, or duplicate orchestrators. Rust timing lives inside `run-benchmarks.sh`.

## Clean up after each benchmark run

- **Delete** any temporary bench scripts you created only to debug a single run (scratch files, copied `bench-*.ts`, extra shell wrappers).
- **Do not** leave standalone `bench-rust.sh`-style helpers; use `run-benchmarks.sh` only.
- **Keep** `bench-<route>.ts` files that are registered in `run-benchmarks.sh` and have a row in `results.md`.
- **Remove** `bench-<route>.ts` when the route is removed or renamed (and drop the `case` + table row; re-run any remaining route so the total recalculates).
- **Do not** commit server logs, `.env` copies, or raw curl output under `performance/`.

## Environment

- Rust: `meetcal-backend/.env` → `CONVEX_URL`
- Rust HTTP: **`cargo run --release`** only (never debug for recorded numbers)
- Rust curl: **`Accept-Encoding: gzip, br`** + `--compressed` (same as real `fetch` clients)
- RN bench: same URL via `CONVEX_URL` or `EXPO_PUBLIC_CONVEX_URL`
- Default: 25 iterations, 1 warmup; report **median** ms
- `all` — runs every route in one session on one release server

## Table columns

| Column | Meaning |
| --- | --- |
| **Rust (ms)** | Median HTTP latency for the Rust route |
| **RN client path (ms)** | Median full in-app path (not Convex-only) |
| **Rust vs RN %** | `((rust_ms - rn_ms) / rn_ms) * 100` — negative = Rust faster |
| **User noticeable** | Whether the delta is likely perceptible in the app (see below) |

### Total (avg) row

On **every** `run-benchmarks.sh` invocation, the script:

- Updates the benchmarked route row (insert or replace).
- Recomputes **`Total (avg)`** at the bottom: arithmetic mean of Rust (ms), RN client path (ms), and Rust vs RN % over all route rows (excluding the total).
- Applies the same noticeable rules to the averaged values.

Small per-route wins still matter for product speed; the total row tracks aggregate Rust vs RN across all benchmarked endpoints.

## User noticeable column

Set automatically by `run-benchmarks.sh`:

- **No** — absolute delta `< 100 ms` **or** `|rust vs rn %| < 25` (either alone is fine)
- **Yes** — **both** ≥100 ms slower/faster **and** ≥25% relative change (avoids flagging tiny ms gaps on fast endpoints)

Tune thresholds here if product expectations change; re-run all route ids after changing thresholds so route rows and the total stay in sync.
