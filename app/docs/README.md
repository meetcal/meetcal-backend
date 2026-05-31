# MeetCal App API — Route Documentation

Internal API for the **`app`** Rust service (`meetcal-backend/app`). Serves the **meetcal-app** mobile client only.

This layer **wraps Convex** — handlers validate auth, shape requests/responses, and call existing Convex queries/mutations. Convex keeps crons, schema, and internal functions unchanged (`convex/crons.ts`, `internal.*`).

Scraper pipelines have their own wrapper service: [`../scrapers/docs/`](../scrapers/docs/).

## Convex references

Each route in the docs below includes a **Convex** line:

```
**Convex:** `module.fn` · `query|mutation` · `{ args }`
```

Use the generated client paths (`api.module.fn`) or HTTP API paths (`module:fn`) when calling from Rust. Forward Clerk JWT to Convex on authenticated routes so Convex auth checks still work.

## Auth

| Caller | Header | Used by |
|---|---|---|
| Signed-in user | `Authorization: Bearer <Clerk JWT>` | `/users/me/*` |
| Public reads | None | Meet data, comp data, clubs |

User routes derive `userId` from the Clerk JWT subject. Never trust a client-supplied user id without matching it to the token.

## What the app uses today

Source: `meetcal-app/lib/database/*`, `meetcal-app/hooks/*`, `meetcal-app/app/*`.

| Area | Writes | Reads |
|---|---|---|
| Saved sessions | upsert, remove, removeAll | getByUser |
| Profile | setAutoUnsave | getForCurrentUser |
| Meets & schedule | — | list for picker, getByName, schedule by meet |
| Start list / attempt estimator | — | athletes, lifting results |
| Comp data screens | — | records, WSO, standards, QT, intl/adaptive/national rankings |
| Club stats | — | clubs, club athletes, meet stats |
| Weightlifting Wrapped | — | athlete search, lifting results search |

## Route index

### User (`user.md`)

| Route | Convex |
|---|---|
| `GET /users/me/saved-sessions` | `savedSessions.getByUser` |
| `PUT /users/me/saved-sessions/:sessionId` | `savedSessions.upsert` |
| `DELETE /users/me/saved-sessions/:sessionId` | `savedSessions.remove` |
| `DELETE /users/me/saved-sessions` | `savedSessions.removeAllForUser` |
| `GET /users/me/preferences` | `userPreferences.getForCurrentUser` |
| `PATCH /users/me/preferences/auto-unsave` | `userPreferences.setAutoUnsaveStartedSessions` |

### Meets & live meet data (`meets.md`)

| Route | Convex |
|---|---|
| `GET /meets` | `meets.listActive` (+ fixed ±3 month filter in Rust) |
| `GET /meets/:name` | `meets.getByName` |
| `GET /meets/:meet/schedule` | `schedule.getByMeet` |
| `GET /meets/:meet/athletes` | `athletes.getByMeet` |
| `GET /meets/:meet/athletes/with-session` | `athletes.getWithSessionByMeet` |
| `GET /meets/:meet/lifting-results` | `liftingResults.getByMeet` |
| `GET /lifting-results/by-names` | `liftingResults.getByNames` |
| `GET /lifting-results/by-names-since` | `liftingResults.getByNamesSince` |
| `GET /lifting-results/year-bests` | `liftingResults.getYearBestsByName` |
| `GET /athletes/search` | `athletes.searchByName` |

### Comp data (`comp-data.md`)

| Route | Convex |
|---|---|
| `GET /records/federations` | `records.listFederations` |
| `GET /records` | `records.getByFederation` |
| `GET /wso-records/wsos` | `wsoRecords.listWsos` |
| `GET /wso-records/:wso` | `wsoRecords.getByWso` |
| `GET /standards` | `standards.getFiltered` |
| `GET /qualifying-totals` | `qualifyingTotals.getAll` |
| `GET /intl-rankings` | `intlRankings.getAll` |
| `GET /lifting-results/national-rankings` | `liftingResults.getNationalRankings` |
| `GET /lifting-results/adaptive` | `liftingResults.getAdaptive` |
| `GET /lifting-results/search` | `liftingResults.searchByNameAndYear` |

### Clubs (`clubs.md`)

| Route | Convex |
|---|---|
| `GET /clubs` | `athletes.listClubs` |
| `GET /clubs/:club/athletes` | `athletes.getByClub`, `meets.getByName` |
| `GET /clubs/:club/meets/:meet/stats` | `athletes.getByClubAndMeet`, `liftingResults.getByMeet`, `athletes.getByMeet`, `liftingResults.getByNames` |

### Infra

| Route | Convex |
|---|---|
| `GET /health` | none |
| `GET /ready` | optional ping (e.g. `meets.listActive` with limit) |

## Common error responses

```json
{ "error": "Unauthenticated" }
{ "error": "Unauthorized" }
{ "error": "Not found" }
{ "error": "Validation failed", "details": ["..."] }
```
