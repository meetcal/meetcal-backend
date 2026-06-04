## `GET /clubs/:club/meets/:meet/stats`

**Why:** Medal counts, PRs, perfect sessions, and per-athlete results for a club at a specific meet.

**App usage:** `fetchClubMeetStats`

**Auth:** None

**Convex:**

1. `athletes.getByClubAndMeet` · `query` · `{ club, meet }`
2. `liftingResults.getByMeet` · `query` · `{ meet }`
3. `athletes.getByMeet` · `query` · `{ meet }`
4. `liftingResults.getByNames` · `query` · `{ names }` — club athlete names from step 1

**Response:** `200`

```json
{
  "totalAthletes": 12,
  "goldMedals": 2,
  "silverMedals": 1,
  "bronzeMedals": 3,
  "totalPRs": 5,
  "perfect6for6": 1,
  "totalWeightLifted": 12450,
  "athleteResults": [
    {
      "name": "Jane Doe",
      "snatchBest": 83,
      "cjBest": 102,
      "total": 185,
      "bodyWeight": 70.5,
      "medal": "gold",
      "isPR": true,
      "perfectLifts": false
    }
  ]
}
```

**Steps:**

1. Call Convex queries above.
2. Filter historical results: exclude `federation === "BWL"`, dates before meet's first result date.
3. Aggregate medals, PRs, perfect lifts in Rust (mirrors `fetch-club-stats.ts`).

**Notes:** Single Rust endpoint replaces four Convex round trips from the app.
