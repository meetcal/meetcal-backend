## Priority routes

### `GET /meets/package?meet=...&history_cutoff_date=...`

**Why:** Main performance win. Offline prefetch currently makes separate requests for meet details, schedule, athletes with sessions, meet lifting results, and athlete history batches.

**Replaces app calls:** `meets.getByName`, `schedule.getByMeet`, `athletes.getWithSessionByMeet`, `liftingResults.getByMeet`, `liftingResults.getByNames` / `getByNamesSince`

**Backend work:** Return one app-ready package for a selected meet:

- meet metadata
- sorted schedule
- athletes already joined to session date/start/weigh-in
- lifting results for athletes in that meet
- optional athlete history since `history_cutoff_date`, grouped by athlete name

**Response shape:**

```json
{
  "meet": {
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
  },
  "schedule": [],
  "athletes": [],
  "liftingResults": [],
  "athleteHistory": {}
}
```

**Notes:** Keep lower-level meet routes, but the RN prefetch path should prefer this route to reduce latency, duplicated client joins, and partial cache states.
