# Comp data routes

Reference data screens under `app/comp-data/*` and offline prefetch in `utils/offline.ts`.

Most responses are cached client-side in AsyncStorage; routes should stay fast and cache-friendly.

---

## `GET /wso-records/wsos`

**Why:** WSO picker on WSO records screen.

**App usage:** `fetch-wso-records.ts` → `fetchWSOList`

**Auth:** None

**Convex:** `wsoRecords.listWsos` · `query` · `{}`

**Response:** `200`

```json
{ "wsos": ["Carolinas", "Florida", "Ohio"] }
```

**Steps:**

1. Call Convex query.

---

## `GET /wso-records/:wso`

**Why:** State/regional record tables.

**App usage:** `fetchWSORecords`, `app/comp-data/wso-records.tsx`

**Auth:** None

**Convex:** `wsoRecords.getByWso` · `query` · `{ wso, ageCategory?, gender? }`

**Query params:** `ageCategory`, `gender` (`Men` | `Women`) — optional

**Response:** `200` — same row shape as `/records`

**Steps:**

1. Call Convex query.
2. App nests into `RecordsData`.

---

## `GET /standards`

**Why:** USAW standard A/B totals by age and gender.

**App usage:** `fetch-standards.ts`, offline prefetch

**Auth:** None

**Convex:** `standards.getFiltered` · `query` · `{ ageCategory?, gender? }`

**Query params:**

- `ageCategory` (optional) — `u15`, `youth`, `junior`, `senior`
- `gender` (optional) — `men` | `women`

**Response:** `200`

```json
{
  "standards": [
    {
      "ageCategory": "senior",
      "gender": "men",
      "weightClass": "89kg",
      "standardA": 240,
      "standardB": 220
    }
  ]
}
```

**Steps:**

1. Call Convex query.
2. App nests into `StandardsData`.

---

## `GET /qualifying-totals`

**Why:** USAMW qualifying total tables.

**App usage:** `fetch-qualifying-totals.ts`, `app/comp-data/new-qualifying-totals.tsx`

**Auth:** None

**Convex:** `qualifyingTotals.getAll` · `query` · `{}`

**Response:** `200`

```json
{
  "rows": [
    {
      "eventName": "National Championships",
      "gender": "Women",
      "ageCategory": "Senior",
      "weightClass": "71kg",
      "qualifyingTotal": 180
    }
  ]
}
```

**Steps:**

1. Call Convex query.
2. App nests and filters client-side.

---

## `GET /intl-rankings`

**Why:** International ranking tables screen.

**App usage:** `fetchIntlRankings.ts`, `app/comp-data/rankings.tsx`

**Auth:** None

**Convex:** `intlRankings.getAll` · `query` · `{}`

**Response:** `200`

```json
{
  "rankings": [
    {
      "meet": "2024 World Championships",
      "ranking": 1,
      "name": "Athlete Name",
      "weightClass": "89kg",
      "total": 380,
      "percentA": 102.5,
      "gender": "Men",
      "ageCategory": "Senior"
    }
  ]
}
```

**Steps:**

1. Call Convex query.
2. App filters/groups client-side.

---

## `GET /lifting-results/national-rankings`

**Why:** USAW national ranking list by weight-class-age string.

**App usage:** `fetch-national-rankings.ts`

**Auth:** None

**Convex:** `liftingResults.getNationalRankings` · `query` · `{ federation, ageCategory }` — app defaults `federation` to `"USAW"`

**Query params:**

- `ageCategory` (required) — e.g. `"Open Men's 89kg"` (stored in lifting result `age` field)
- `federation` (optional, default `USAW`)

**Response:** `200`

```json
{
  "rankings": [{ "id": 0, "name": "Jane Doe", "total": 285 }]
}
```

**Steps:**

1. Call Convex query.
2. App deduplicates by name and assigns display ids.

---

## `GET /lifting-results/adaptive`

**Why:** Adaptive division record-style view derived from best adaptive meet totals.

**App usage:** `fetch-adaptive-records.ts`, `app/comp-data/adap-records.tsx`

**Auth:** None

**Convex:** `liftingResults.getAdaptive` · `query` · `{ excludeFederation?: string }` — app sends `"BWL"`

**Query params:** `excludeFederation` (optional)

**Response:** `200` — `{ "results": [...] }`

**Steps:**

1. Call Convex query.
2. App extracts weight class from `age` string and keeps best total per class/gender.

---

## `GET /lifting-results/search`

**Why:** Fallback search when exact name lookup returns no Wrapped results for a calendar year.

**App usage:** `weightlifting-wrapped.tsx`

**Auth:** None

**Convex:** `liftingResults.searchByNameAndYear` · `query` · `{ query: string, startDate?: string, endDate?: string }`

**Query params:**

- `q` — partial name (maps to `query`)
- `startDate` — `YYYY-MM-DD` inclusive
- `endDate` — `YYYY-MM-DD` exclusive

**Response:** `200` — `{ "results": [...] }`

**Steps:**

1. Call Convex query.
2. App caps at 600 rows and applies word matching.
