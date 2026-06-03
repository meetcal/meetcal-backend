# RN client path benchmarks

**RN column = full in-app work replaced by the API**, not a single Convex call.

Each `bench-*.ts` runs `client-paths/*` which mirrors:

1. Network fetch (`fetchFresh` / screen-equivalent queries)
2. Map, filter, dedupe, sort (handler + screen `useMemo`)
3. Persist to offline cache when the app does on load
4. Final shape passed to UI (e.g. Wrapped stats, table rows)

Requires `import "../lib/bench-preload"` first so `@/lib/convex`, `@/lib/networkUtils`, and offline cache behave under Bun.
