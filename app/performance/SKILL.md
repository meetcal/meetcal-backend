---
name: api-performance-benchmark
description: >-
  Benchmark MeetCal Rust API routes vs the full meetcal-app client path. Use when adding or
  changing routes in meetcal-backend/app, running performance/scripts/run-benchmarks.sh,
  updating performance/results.md, or the user mentions API latency, benchmarks, or Rust vs RN
  speed. Run on every new route; Total (avg) row recalculates each run.
---

# API performance benchmarking

Compare **Rust `meetcal-backend/app`** HTTP handlers to the **meetcal-app code path the UI actually runs** — from Convex fetch through transforms to the data shape the screen displays.

**Not** a single Convex call. **Not** Rust-handler-only unless the app truly does nothing else.

See also: `meetcal-backend/app/performance/BENCHMARK-RN-PATH.md`.

## Agent: run this on every new route

When you add or materially change a Rust API route, **always** finish by running the benchmark for that route.

1. Implement the Rust route and register it in `lib.rs`.
2. Add `meetcal-app/scripts/performance/client-paths/<route>.ts` with the **full** RN path (see below).
3. Add thin `meetcal-app/scripts/performance/bench-<route>.ts` (preload + `runMedianBench`).
4. Add route id to `ALL_ROUTES` and a `case` in `performance/scripts/run-benchmarks.sh`.
5. Run:

   ```bash
   cd meetcal-backend/app && bash performance/scripts/run-benchmarks.sh '<route-id>'
   ```

   Or after a batch:

   ```bash
   cd meetcal-backend/app && bash performance/scripts/run-benchmarks.sh all
   ```

6. Confirm `performance/results.md` has the route row and updated **`Total (avg)`**.
7. Commit `results.md` only when the user asks.

Re-run when Rust handler **or** RN client-path logic changes.

**Do not** hand-edit per-route ms or the total row — `run-benchmarks.sh` writes them.

## What to measure

| Layer | Measure |
| --- | --- |
| **Rust** | End-to-end HTTP (`GET`): Convex + Rust handler + JSON (release build, compressed). |
| **RN** | Full client path in `client-paths/<route>.ts`: all Convex calls the screen triggers, plus map/filter/dedupe/sort, screen slice (`useMemo` equivalent), and final display payload (e.g. Wrapped stats, table rows). |

### Out of scope (do not time these in RN bench)

- React render / reconciliation
- `useMutableResource` hook bookkeeping (unless you explicitly add a bench for it later)
- Navigation, alerts, loading spinners

### In scope (required when the app does them)

- Multiple Convex queries (exact + fallback, `getAll` + client filter, etc.)
- Mapping Convex rows to app types
- Client-side filter/dedupe/sort the screen relies on
- Final aggregation (e.g. `calculateWrappedStats` for Wrapped)

## File layout (keep only this)

```
meetcal-app/scripts/performance/
  lib/
    bench-preload.ts      # import "./convex-http" only
    convex-http.ts        # ConvexHttpClient + loadEnv (no React)
    bench-runner.ts       # median loop (ITERATIONS, WARMUP)
  client-paths/
    <route>.ts            # full RN path — source of truth for RN column
  bench-<route>.ts        # thin wrapper per route

meetcal-backend/app/performance/
  scripts/run-benchmarks.sh
  results.md
  BENCHMARK-RN-PATH.md
```

Do **not** add duplicate runners, `bench-rust.sh`, or Convex-only benches.

### Thin `bench-<route>.ts` template

```ts
import "./lib/bench-preload";
import { runXxxClientPath } from "./client-paths/<route>";
import { runMedianBench } from "./lib/bench-runner";

await runMedianBench(runXxxClientPath);
```

### `client-paths/<route>.ts` rules

1. Read the **screen** (e.g. `app/comp-data/*.tsx`) and **`lib/database/fetch-*.ts`** the screen uses.
2. Copy the logic the app runs **before** data is shown — not the Rust handler unless they match.
3. Use `convexQuery` from `../lib/convex-http` and `api` from `../../../convex/_generated/api`.
4. End with the variable the UI consumes (`void` it so the compiler keeps the work).
5. Add a one-line comment at top: `// Mirrors: <screen> + <fetch module>`.

**Never** import `@/lib/convex` in benches — it pulls `ConvexReactClient` and breaks Bun.

## Route ids (current)

| Route id | HTTP (query params) | Client path file | App source to mirror |
| --- | --- | --- | --- |
| `meets` | `GET /meets` | `meets.ts` | `listActive` + 3-month filter + sort |
| `meet-details` | `GET /meet-details?meet=` | `meet-details.ts` | `meets.getByName` + response map |
| `meets/schedule` | `GET /meets/schedule?meet=` | `meet-schedule.ts` | `schedule.getByMeet` + Rust sort |
| `meets/athletes` | `GET /meets/athletes?meet=` | `meet-athletes.ts` | `athletes.getByMeet` + map + name sort |
| `clubs` | `GET /clubs` | `clubs.ts` | `athletes.listClubs` |
| `records` | `GET /records?recordType&gender&ageCategory` | `records.ts` | `fetch-records`: full federation `getByFederation` + `mapRowsToRecordsData` + screen slice |
| `wso` | `GET /wso` | `wso.ts` | `wsoRecords.listWsos` |
| `wso-records` | `GET /wso-records?wso&gender&ageCategory` | `wso-records.ts` | `fetch-wso-records`: `getByWso` + group/sort + screen slice |
| `standards` | `GET /standards?gender&ageCategory` | `standards.ts` | `fetch-standards`: `getFiltered` {} + map + screen slice |
| `qualifying-totals` | `GET /qualifying-totals?eventName&...` | `qualifying-totals.ts` | `qualifyingTotals.getAll` + build tree + slice |
| `intl-rankings` | `GET /intl-rankings?meet&gender&ageCategory` | `intl-rankings.ts` | `intlRankings.getAll` + screen filter/sort (**not** `getFiltered` alone) |
| `nat-rankings` | `GET /nat-rankings?federation&ageCategory` | `nat-rankings.ts` | `getNationalRankings` + `mapRankings` (**not** Rust max-total dedupe) |
| `adaptive` | `GET /adaptive?excludeFederation&gender` | `adaptive.ts` | `fetch-adaptive-records` gender path (**not** Rust regex/year filter) |
| `search` | `GET /search?query&startDate&endDate` | `search.ts` | `weightlifting-wrapped` `searchAthlete`: exact → fallback → word filter → map → **`calculateWrappedStats`** |

Env overrides: `BENCH_MEET_NAME`, `BENCH_SEARCH_QUERY`, `BENCH_NAT_AGE_CATEGORY`, `BENCH_ADAPTIVE_GENDER`, etc. (see `configure_route` in `run-benchmarks.sh`).

## Workflow for a new route

1. Rust handler in `src/routes/`.
2. `client-paths/<route>.ts` — implement full RN path (steps above).
3. `bench-<route>.ts` — thin wrapper.
4. `ALL_ROUTES` + `case` in `run-benchmarks.sh` (URL-encode query values; watch apostrophes in bash defaults).
5. `bash performance/scripts/run-benchmarks.sh <route-id>` (release server managed by script).
6. Verify `results.md`.

### Checklist before claiming RN bench is done

- [ ] Traced screen → fetch module → any inline screen logic (`useMemo`, stats helpers)
- [ ] RN bench uses **same** Convex function(s) as the app (not the Rust function if they differ)
- [ ] Includes client transforms after fetch
- [ ] Includes final display shape (stats object, table rows, name list)
- [ ] Runs under Bun: `cd meetcal-app && bun scripts/performance/bench-<route>.ts` prints one median number

## Environment

- `meetcal-backend/.env` → `CONVEX_URL` (loaded by `convex-http.ts` and Rust)
- Rust: **`cargo run --release`** only; curl uses `Accept-Encoding: gzip, br`
- RN: `bun scripts/performance/bench-*.ts` from `meetcal-app`
- Defaults: `ITERATIONS=25`, `WARMUP=1`, median ms
- `BENCH_MANAGE_SERVER=0` if release server already on port 3000

## `results.md` columns

| Column | Meaning |
| --- | --- |
| **Rust (ms)** | Median HTTP latency |
| **RN client path (ms)** | Median full `client-paths` work |
| **Rust vs RN %** | `((rust - rn) / rn) * 100` — negative = Rust faster |
| **User noticeable** | Auto: Yes only if **both** ≥100 ms delta **and** ≥25% relative |

**Total (avg)** — mean across all route rows; recomputed every `run-benchmarks.sh` run.

Remove stale route rows (old path-style ids like `meets/:name`) when URLs change; re-run `all` so the total stays correct.

## Clean up

- Delete scratch/debug scripts after a run
- Keep registered `bench-*.ts` + matching `client-paths/*.ts`
- Remove both when a route is deleted; re-run `all` for total
- Do not commit logs or raw curl output under `performance/`
