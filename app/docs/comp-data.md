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
