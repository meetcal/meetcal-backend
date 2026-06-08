# React Native app route gaps

Routes needed by the MeetCal React Native app that are not covered by `user.md`.

Important implementation rule: all data handling, joins, aggregation, filtering, sorting, mutation side effects, and response shaping should happen in this backend. The RN app should receive completed, screen-ready data and should not need to recreate Convex query logic client-side.

---

## New routes needed

### `GET /meets/active`

**Why:** The app currently calls `meets.listActive` to populate meet selection and offline cache.

**App usage:** `lib/database/meet-manager.ts` · `fetchMeetsFresh`

**Backend work:** Return full meet objects, not just meet names. Include status and venue/time fields already needed by the RN `Meet` type.

**Response shape:**

```json
{
  "meets": [
    {
      "id": "meet_nationals_2026",
      "name": "2026 USA Weightlifting National Championships, Powered by Rogue Fitness",
      "federation": "USAW",
      "status": "upcoming",
      "startDate": "2026-06-20",
      "endDate": "2026-06-28",
      "timeZone": "America/Chicago",
      "venueName": "Greater Columbus Convention Center",
      "venueStreet": "400 N High St",
      "venueCity": "Columbus",
      "venueState": "OH",
      "venueZip": "43215"
    }
  ]
}
```

**Notes:** Current `GET /meets` returns only `{ "names": [...] }` for non-completed meets in the next 3 months. Keep it if useful, but the RN app needs this full active-meet route.

---

### `GET /meets/by-name?name=...`

**Why:** The app currently calls `meets.getByName` for selected meet config and club meet status checks.

**App usage:** `lib/database/meet-manager.ts` · `fetchMeetByName`; `lib/database/fetch-club-stats.ts` · `fetchAthletesByClubFresh`

**Backend work:** Return one full meet object, including `status`. The route should use `name` as the query param, or we should update the app to call the existing `meet` param consistently.

**Response shape:**

```json
{
  "id": "meet_ohio_2026",
  "name": "2026 Ohio WSO Championships",
  "federation": "USAW",
  "status": "completed",
  "startDate": "2026-05-01",
  "endDate": "2026-05-03",
  "timeZone": "America/New_York",
  "venueName": "Ohio Expo Center",
  "venueStreet": "717 E 17th Ave",
  "venueCity": "Columbus",
  "venueState": "OH",
  "venueZip": "43211"
}
```

**Notes:** Existing `GET /meets/details?meet=...` is close, but it does not return `status` and its field casing is not app-ready.

---

### `GET /athletes/search?query=...`

**Why:** The app needs athlete-name suggestions, not full result rows.

**App usage:** `lib/database/queries.ts` · `searchAthletesByName`; `app/comp-data/weightlifting-wrapped.tsx` · `fetchSuggestions`

**Backend work:** Search `lifting_results.name`, return unique names sorted for display, and cap the response.

**Response shape:**

```json
{
  "names": [
    "Alexander Nordstrom"
  ]
}
```

**Notes:** Existing `GET /search` requires a date range and returns lifting result rows. Keep `/search` for wrapped result search, and add this route for suggestions.

---

### `GET /lifting-results/by-names-since?names=...&cutoff_date=...`

**Why:** Attempt estimator needs recent history for a batch of athlete names.

**App usage:** `app/shared-screens/attempt-estimator.tsx`

**Backend work:** Accept comma-separated names and a caller-provided cutoff date. Return complete lifting result rows in app-ready shape.

**Response shape:**

```json
[
  {
    "federation": "USAW",
    "meet": "2026 Adaptive Men 85kg National Championship",
    "date": "2026-02-01",
    "name": "Adaptive Test Athlete",
    "age": "Adaptive Men 85kg",
    "body_weight": 84.5,
    "snatch1": 35.0,
    "snatch2": 40.0,
    "snatch3": 0.0,
    "snatch_best": 40.0,
    "cj1": 45.0,
    "cj2": 50.0,
    "cj3": 0.0,
    "cj_best": 50.0,
    "total": 90.0,
    "adaptive": true
  }
]
```

**Notes:** Existing `GET /lifting-results/recent` is fixed to the last 2 years. This route should let the app pass the cutoff date.

---

### `GET /lifting-results/year-bests?name=...&cutoff_date=...`

**Why:** Start list attempt cards need one athlete's best snatch, clean and jerk, and total since a cutoff.

**App usage:** `lib/start-list-api.ts` · `getLastYearBests`

**Backend work:** Calculate max values in SQL and return only the completed bests object.

**Response shape:**

```json
{
  "bestSnatch": 40.0,
  "bestCJ": 50.0,
  "bestTotal": 90.0
}
```

**Notes:** Existing `GET /lifting-results/year` returns raw rows for current date minus 1 year. The app should not need to calculate maxes itself.

---

## Existing routes that need updates

### `GET /data/records`

**Current backend:** Requires `record_type`, `gender`, and `age_category`.

**App usage:** `lib/database/fetch-records.ts`

**Needed update:** Support app-ready federation data and list values:

- `GET /data/records/federations` returns `{ "federations": ["USAW", "USAMW", "IWF", "UMWF"] }`.
- `GET /data/records?record_type=USAW` returns all records for that federation.
- Optional `age_category` and `gender` filters should narrow the result when present.

**Backend-owned data handling:** Sort weight classes and return rows with consistent casing. The app should not need to fetch an entire table only to discover age groups or federations.

---

### `GET /data/standards`

**Current backend:** Requires `gender` and `age_category`.

**App usage:** `lib/database/fetch-standards.ts`

**Needed update:** Allow no params to return all standards, with optional `age_category` and `gender` filters.

**Backend-owned data handling:** Return complete rows sorted by age category, gender, and weight class.

---

### `GET /data/qualifying-totals`

**Current backend:** Requires `event_name`, `gender`, and `age_category`.

**App usage:** `lib/database/fetch-qualifying-totals.ts`

**Needed update:** Allow no params to return all qualifying totals, with optional `event_name`, `age_category`, `gender`, and `weight_class` filters.

**Backend-owned data handling:** Return a complete filtered set so the app does not need to fetch all data and build nested lookup maps itself.

---

### `GET /data/intl-rankings`

**Current backend:** Requires `meet`, `gender`, and `age_category`.

**App usage:** `lib/database/fetchIntlRankings.ts`

**Needed update:** Allow no params to return all international rankings, with optional `meet`, `gender`, and `age_category` filters.

**Backend-owned data handling:** Sort by meet, gender, age category, then ranking. Return rows in the final casing the app will consume.

---

### `GET /clubs/meet-stats`

**Current backend:** Route exists, but the SQL aggregation does not yet match the RN app's club stats logic.

**App usage:** `lib/database/fetch-club-stats.ts`; `app/comp-data/club-results/results.tsx`

**Needed update:** Return the fully computed club meet report:

```json
{
  "totalAthletes": 1,
  "goldMedals": 0,
  "silverMedals": 0,
  "bronzeMedals": 0,
  "totalPRs": 0,
  "perfect6for6": 0,
  "totalWeightLifted": 0.0,
  "athleteResults": []
}
```

**Backend-owned data handling:** The backend must calculate:

1. Club athletes for the meet.
2. Matching lifting results for those athletes.
3. Historical best total before the meet date for PR detection.
4. Six-for-six attempts.
5. Total lifted.
6. Medal counts for snatch, clean and jerk, and total within the athlete's weight class, matching the current RN logic.

**Notes:** The current route ranks by total only, partitioned by `age`. The app logic counts snatch medals, clean and jerk medals, and total medals separately within `weightClass`.

---

### `GET /clubs/athletes`

**Current backend:** Returns completed-meet club athletes, but omits `member_id`.

**App usage:** `lib/database/fetch-club-stats.ts`; `types/club.ts`

**Needed update:** Include `member_id` in the response, since `AthleteClub` expects it.

**Backend-owned data handling:** Keep the completed-meet filtering server-side.

---

## Routes already covered well enough

These routes have equivalents that can support the RN app once the app switches away from direct Convex calls:

- `GET /meets/schedule`
- `GET /meets/athletes`
- `GET /meets/athletes-sessions`
- `GET /clubs`
- `GET /data/wso/`
- `GET /data/wso/records`
- `GET /data/nat-rankings`
- `GET /data/adaptive`
- `GET /lifting-results`
- `GET /lifting-results/by-names`
- `GET /search`

Some may still need casing adapters depending on whether the RN migration keeps camelCase app types or accepts snake_case API rows.
