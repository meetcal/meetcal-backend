# Scraper routes

**Auth:** `X-Scraper-Secret` on every request.

Each route validates the secret in Rust, then calls the matching Convex **action** (same as Python scrapers today). Actions delegate to **internal mutations** listed inline.

---

## `POST /lifting-results`

**Why:** Sport80/BWL result sync.

**Convex:** `scraperIngestion.ingestLiftingResult` · `action` → `internal.liftingResults.upsertLiftingResult`

**Request:**

```json
{
  "legacyId": 123,
  "eventId": "sport80-id",
  "meet": "2025 Local Meet",
  "date": "2025-05-31",
  "name": "Jane Doe",
  "age": "71kg Women Senior",
  "bodyWeight": 70.5,
  "snatch1": 75, "snatch2": 80, "snatch3": 83, "snatchBest": 83,
  "cj1": 95, "cj2": 100, "cj3": 102, "cjBest": 102,
  "total": 185,
  "adaptive": false,
  "federation": "USAW"
}
```

**Response:** `{ "id": "...", "wasInsert": true, "wasChanged": true }`

**Steps:**
1. Validate scraper secret.
2. Call Convex action with body + `scraperSecret`.

---

## `POST /records`

**Why:** USAW/USAMW federation record scrapers.

**Convex:** `scraperIngestion.ingestRecord` · `action` → `internal.records.upsertRecord`

**Request:** `recordType`, `ageCategory`, `gender`, `weightClass`, optional `snatchRecord`, `cjRecord`, `totalRecord`

**Response:** `{ "id", "wasInsert", "wasChanged" }`

---

## `PUT /records/iwf`

**Why:** IWF world records full refresh.

**Convex:** `scraperIngestion.replaceIWFRecords` · `action` → `internal.records.deleteByRecordType` + `internal.records.upsertRecord` (per row)

**Request:** `{ "records": [ ...record objects... ] }`

**Response:** `{ "deleted": true, "inserted": 42 }`

---

## `POST /qualifying-totals`

**Convex:** `scraperIngestion.ingestQualifyingTotal` · `action` → `internal.qualifyingTotals.upsertQualifyingTotal`

**Request:** `eventName`, `gender`, `ageCategory`, `weightClass`, `qualifyingTotal`

**Response:** `{ "id", "wasInsert" }`

---

## `POST /standards`

**Convex:** `scraperIngestion.ingestStandard` · `action` → `internal.standards.upsertStandard`

**Request:** `ageCategory`, `gender`, `weightClass`, `standardA`, `standardB`

**Response:** `{ "id", "wasInsert", "wasChanged" }`

---

## `POST /athletes`

**Why:** Entry list / session assignment scrapers.

**Convex:** `scraperIngestion.ingestAthlete` · `action` → `internal.athletes.upsertAthlete`

**Request:** `memberId`, `name`, `age`, `club`, optional `wso`, `gender`, `weightClass`, `entryTotal`, optional `sessionNumber`, `sessionPlatform`, `meet`, `adaptive`

**Response:** `{ "id", "wasInsert" }`

---

## `DELETE /athletes?meet=`

**Why:** Clear meet entries before re-import.

**Convex:** `scraperIngestion.deleteAthletesByMeet` · `action` → `internal.athletes.deleteByMeet` · `{ meet }`

**Response:** `{ "deleted": 12 }`

---

## `POST /session-schedule`

**Convex:** `scraperIngestion.ingestSessionSchedule` · `action` → `internal.schedule.upsertSessionSchedule`

**Request:** `date`, `sessionId`, `startTime`, `weighInTime`, `platform`, `weightClass`, `meet`

**Response:** `{ "id", "wasInsert" }`

---

## `POST /wso-records`

**Convex:** `scraperIngestion.ingestWSORecord` · `action` → `internal.wsoRecords.upsertWSORecord`

**Request:** `wso`, `ageCategory`, `gender`, `weightClass`, optional lift records

**Response:** `{ "id", "wasInsert", "wasChanged" }`

---

## `PUT /wso-records/:wso`

**Why:** Full replace for a WSO (Illinois PDF scraper).

**Convex:** `scraperIngestion.replaceWSORecordSet` · `action` → `internal.wsoRecords.replaceByWso`

**Request:** `{ "records": [ ... ] }` — `:wso` path param maps to action arg `wso`

**Response:** `{ "inserted", "updated", "unchanged", "deleted" }`

---

## `POST /meets`

**Convex:** `scraperIngestion.ingestMeet` · `action` → `internal.meets.upsertMeet`

**Request:** `name`, venue fields, `timeZone`, `startDate`, `endDate`, optional `status`, `federation`

**Response:** `{ "id", "wasInsert" }`

---

## `POST /intl-rankings`

**Convex:** `scraperIngestion.ingestIntlRanking` · `action` → `internal.intlRankings.upsertIntlRanking`

**Request:** optional `legacyId`, `meet`, `ranking`, `name`, `weightClass`, `total`, `percentA`, `gender`, `ageCategory`

**Response:** `{ "id", "wasInsert" }`

---

## `PUT /intl-rankings`

**Why:** Bulk replace entire rankings table.

**Convex:** `scraperIngestion.replaceAllIntlRankings` · `action` → `internal.intlRankings.deleteAll` + `internal.intlRankings.upsertIntlRanking` (per row)

**Request:** `{ "rankings": [ ... ] }`

**Response:** `{ "inserted": N }`

---

## `PUT /intl-rankings/groups/:meet/:gender/:ageCategory`

**Why:** Incremental group sync from intl rankings scraper.

**Convex:** `scraperIngestion.replaceIntlRankingsForGroup` · `action` → `internal.intlRankings.replaceByMeetGenderAge`

**Request:** `{ "rankings": [ ... ] }` — path params map to `meet`, `gender`, `ageCategory`

**Response:** `{ "inserted", "updated", "unchanged", "deleted" }`

---

## `POST /intl-rankings/prune-groups`

**Convex:** `scraperIngestion.deleteMissingIntlRankingGroups` · `action` → `internal.intlRankings.deleteGroupsNotIn`

**Request:** `{ "groups": [ { "meet", "gender", "ageCategory" } ] }`

**Response:** `{ "deletedGroups": [ ... ] }`

---

## `PUT /saved-sessions/:sessionId`

**Why:** Legacy Supabase → Convex migration only. Retire when migration is done.

**Convex:** `savedSessions.upsertFromIngestion` · `mutation` · `{ scraperSecret, sessionId, userId, meet, sessionNumber, platform, ... }`

**Request:** `userId`, `meet`, `sessionNumber`, `platform`, optional saved-session fields

**Response:** `{ "sessionId", "updatedAt" }`
