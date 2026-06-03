# Meets & live meet data

## Used by meet picker, schedule tab, start list, attempt estimator, saved sessions, and meet prefetch/offline cache

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
