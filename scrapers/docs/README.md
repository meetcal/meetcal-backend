# MeetCal Scraper API — Route Documentation

Internal HTTP API for the **`scrapers`** Rust service (`meetcal-backend/scrapers`). Called by Python/JS pipelines in `meetcal-app/scrapers/*`, not by the mobile app.

Wraps Convex `scraperIngestion:*` actions — validates `X-Scraper-Secret`, then forwards to the same Convex functions scrapers call today. Does not replace Convex crons or schema.

The mobile app API is documented separately in [`../app/docs/`](../app/docs/).

## Convex references

Each route in [routes.md](./routes.md) includes:

```
**Convex:** `scraperIngestion.fn` · `action` → `internal.module.mutation`
```

Call the **action** from Rust (same as Python `client.action("scraperIngestion:ingestLiftingResult", ...)`). Actions validate `scraperSecret` and run internal mutations.

## Auth

| Header | Value |
|---|---|
| `X-Scraper-Secret` | `SCRAPER_SECRET` env var |

## Route index

| Route | Convex action |
|---|---|
| `POST /lifting-results` | `scraperIngestion.ingestLiftingResult` |
| `POST /records` | `scraperIngestion.ingestRecord` |
| `PUT /records/iwf` | `scraperIngestion.replaceIWFRecords` |
| `POST /qualifying-totals` | `scraperIngestion.ingestQualifyingTotal` |
| `POST /standards` | `scraperIngestion.ingestStandard` |
| `POST /athletes` | `scraperIngestion.ingestAthlete` |
| `DELETE /athletes?meet=` | `scraperIngestion.deleteAthletesByMeet` |
| `POST /session-schedule` | `scraperIngestion.ingestSessionSchedule` |
| `POST /wso-records` | `scraperIngestion.ingestWSORecord` |
| `PUT /wso-records/:wso` | `scraperIngestion.replaceWSORecordSet` |
| `POST /meets` | `scraperIngestion.ingestMeet` |
| `POST /intl-rankings` | `scraperIngestion.ingestIntlRanking` |
| `PUT /intl-rankings` | `scraperIngestion.replaceAllIntlRankings` |
| `PUT /intl-rankings/groups/:meet/:gender/:ageCategory` | `scraperIngestion.replaceIntlRankingsForGroup` |
| `POST /intl-rankings/prune-groups` | `scraperIngestion.deleteMissingIntlRankingGroups` |
| `PUT /saved-sessions/:sessionId` | `savedSessions.upsertFromIngestion` (mutation, not action) |

See [routes.md](./routes.md) for request/response shapes and internal mutation targets.

## Infra

| Route | Convex |
|---|---|
| `GET /health` | none |
| `GET /ready` | optional ping |

## Callers (today)

| Pipeline | Routes |
|---|---|
| `scrapers/usaw/sport80_api/*`, `scrapers/bwl/sport80_api/*` | `POST /lifting-results` |
| `scrapers/usaw/records_scraper/*`, `scrapers/iwf/world-records/*` | `POST /records`, `PUT /records/iwf` |
| `scrapers/usaw/standards_scraper/*` | `POST /standards` |
| `scrapers/usamw/qt/*` | `POST /qualifying-totals` |
| `scrapers/usamw/meets/*`, `scrapers/usaw/meet_to_supabase/scripts/sync-*.js` | `POST /meets` |
| `scrapers/usaw/owlcms_schedule_scraper/*`, `scrapers/usaw/entry_scraper/*` | `POST /session-schedule`, `POST /athletes` |
| `scrapers/usaw/wso_sheets_scraper/*` | `POST /wso-records`, `PUT /wso-records/:wso` |
| `scrapers/usaw/rankings_scraper/intl_rankings_scraper.py` | intl-rankings routes |
