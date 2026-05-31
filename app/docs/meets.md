# Meets & live meet data

Used by meet picker, schedule tab, start list, attempt estimator, saved sessions, and meet prefetch/offline cache.

---

## `GET /meets`

**Why:** Populate meet list for the schedule tab picker.

**App usage:** `fetchMeetsFresh` → `useUpcomingMeets` → `MeetSelectionModal`

**Auth:** None (public read)

**Convex:** `meets.listActive` · `query` · `{}` — filter to ±3 month window in Rust (same as `useUpcomingMeets` default)

**Response:** `200`

```json
{
  "names": ["2025 Nationals", "2025 Local Championships"]
}
```

**Steps:**
1. Call `meets.listActive`.
2. Filter to meets overlapping the date window.
3. Sort by start date ascending.
4. Return names (or thin `{ name, start, end, timeZone }[]` if client still needs dates without `GET /meets/:name`).

**Notes:**
- Completed meets are never shown; Convex cron runs `meetStatusJob.run` → `meets.markCompletedMeets`.
- `GET /meets/:name` covers one-off lookups (e.g. club stats on a finished meet).

---

## `GET /meets/:name`

**Why:** Resolve meet metadata (timezone, dates, venue) on select or cache miss.

**App usage:** `fetchMeetByName`, club stats meet status lookup

**Auth:** None

**Convex:** `meets.getByName` · `query` · `{ name }`

**Response:** `200` — full meet object mapped to app `Meet` shape, or `404`

**Steps:**
1. URL-decode `:name`.
2. Call Convex query; map fields or return 404.

---

## `GET /meets/:meet/schedule`

**Why:** Session/platform schedule for schedule tab, saved-session notifications, offline sync.

**App usage:** `lib/database/queries.ts` → `fetchSchedule`, `SyncManager`

**Auth:** None

**Convex:** `schedule.getByMeet` · `query` · `{ meet }`

**Response:** `200`

```json
{
  "rows": [
    {
      "date": "2025-05-31",
      "sessionId": 3,
      "startTime": "10:00",
      "weighInTime": "08:00",
      "platform": "Red",
      "weightClass": "73kg",
      "meet": "2025 Nationals"
    }
  ]
}
```

**Steps:**
1. Call Convex query.
2. Sort by date, sessionId, platform in Rust if needed.

**Notes:** App times out after 4s and falls back to empty/cached schedule.

---

## `GET /meets/:meet/athletes`

**Why:** Start list athlete list when session join data is unavailable.

**App usage:** `fetchAthletes`, `StartListContent` fallback

**Auth:** None

**Convex:** `athletes.getByMeet` · `query` · `{ meet }`

**Response:** `200`

```json
{
  "athletes": [
    {
      "memberId": "12345",
      "name": "Jane Doe",
      "age": 28,
      "club": "ABC Weightlifting",
      "wso": "Mountain South",
      "gender": "Women",
      "weightClass": "71kg",
      "entryTotal": 180,
      "adaptive": false,
      "sessionNumber": 3,
      "sessionPlatform": "Red"
    }
  ]
}
```

**Steps:**
1. Call Convex query; return athlete rows.

---

## `GET /meets/:meet/athletes/with-session`

**Why:** Start list and attempt estimator need athletes joined with schedule timing.

**App usage:** `fetchAthletesWithSession`, `useMeetAthletes`, `attempt-estimator.tsx`

**Auth:** None

**Convex:** `athletes.getWithSessionByMeet` · `query` · `{ meet }`

**Response:** `200`

```json
{
  "athletes": [
    {
      "memberId": "12345",
      "name": "Jane Doe",
      "age": 28,
      "club": "ABC Weightlifting",
      "wso": "Mountain South",
      "gender": "Women",
      "weightClass": "71kg",
      "entryTotal": 180,
      "adaptive": false,
      "sessionNumber": 3,
      "sessionPlatform": "Red",
      "scheduleRow": {
        "date": "2025-05-31",
        "startTime": "10:00",
        "weighInTime": "08:00",
        "platform": "Red"
      }
    }
  ]
}
```

**Steps:**
1. Call Convex query (join of athletes + session_schedule is done in Convex).
2. Map to app `LiftResult` shape client-side or in Rust.

---

## `GET /meets/:meet/lifting-results`

**Why:** Meet-scoped results for start list and meet prefetch.

**App usage:** `fetchLiftingResultsForMeet`, `meet-manager.prefetchMeetData`

**Auth:** None

**Convex:** `liftingResults.getByMeet` · `query` · `{ meet }`

**Response:** `200` — `{ "results": [...] }`

**Steps:**
1. Call Convex query; map camelCase → app snake_case if needed.

---

## `GET /lifting-results/by-names`

**Why:** Athlete history across meets — club stats PRs, wrapped, generic history.

**App usage:** `fetchAthleteHistoryForNames`, `fetchAllResultsForName`, `weightlifting-wrapped.tsx`, club stats

**Auth:** None

**Convex:** `liftingResults.getByNames` · `query` · `{ names: string[] }`

**Query params:** `names` — repeated or comma-separated

**Response:** `200` — `{ "results": [...] }`

**Steps:**
1. Parse name list; return `[]` if empty.
2. Call Convex query (sort by date desc is done in Convex).

---

## `GET /lifting-results/by-names-since`

**Why:** Attempt estimator loads ~2 years of history for meet athletes.

**App usage:** `attempt-estimator.tsx`

**Auth:** None

**Convex:** `liftingResults.getByNamesSince` · `query` · `{ names: string[], cutoffDate: string }`

**Query params:**
- `names` — athlete names
- `cutoffDate` — ISO date `YYYY-MM-DD`

**Response:** `200` — `{ "results": [...] }`

**Steps:**
1. Call Convex query with parsed params.

---

## `GET /lifting-results/year-bests`

**Why:** Start list shows last-12-month snatch/CJ/total bests per athlete.

**App usage:** `lib/start-list-api.ts` → `getLastYearBests`

**Auth:** None

**Convex:** `liftingResults.getYearBestsByName` · `query` · `{ name: string, cutoffDate: string }`

**Query params:**
- `name` — athlete name
- `cutoffDate` — ISO date one year ago

**Response:** `200` — `{ "results": [...] }`

**Steps:**
1. Call Convex query.
2. App derives max snatch/CJ/total client-side.

---

## `GET /athletes/search`

**Why:** Autocomplete athlete names (Wrapped search, 4+ chars).

**App usage:** `searchAthletesByName`, `weightlifting-wrapped.tsx`

**Auth:** None

**Convex:** `athletes.searchByName` · `query` · `{ query: string }`

**Query params:** `q` — search string (min 4 chars in app)

**Response:** `200`

```json
{ "names": ["Jane Doe", "Jane Smith"] }
```

**Steps:**
1. Call Convex query with `q` as `query` arg.
2. App applies additional word-filter client-side.
