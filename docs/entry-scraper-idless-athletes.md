# Entry scraper: athletes without a membership number

Some Sport80 entry lists omit the `member_id` column for an athlete. The
entry scraper (`scrapers/usaw/entry_scraper/csv_scraper.js`) used to fill the
gap with `Math.random()`, so the writer's identity
`stable_id("athlete", meet, member_id, name)` never matched on the next
nightly run and the same person gained one `athletes` row per night.

## Current behaviour

- The scraper mints a deterministic placeholder, `noid:<slug of the normalised
  name>` (`placeholderMemberId` in `csv_scraper.js`; the same rule is
  `placeholder_member_id` in `scrapers/common/normalize.py`).
- `postgres_writer.upsert_athlete` treats a blank or `noid:` member id as
  "id-less": the row's `convex_id` is derived from `(meet, normalised name)`
  and the fallback lookup is `(meet, id-less member_id, normalised name)`,
  using the same `lower(btrim(regexp_replace(name, '\s+', ' ', 'g')))`
  expression as `idx_athletes_name_normalized`. Two ingests of the same
  id-less athlete update one row, whatever the name's casing or spacing.
- An athlete with a real membership number keeps the old identity, so two
  different athletes who share a name are still two rows.

## One-off cleanup of rows that already have random ids

Rows written before this change carry random nine-digit ids that look like
real membership numbers. `scrapers/common/dedupe_idless_athletes.py`
collapses the groups it can prove are duplicates:

- same meet, normalised name and gender, more than one row;
- every member id is blank, `noid:`, or a nine-digit number in the range the
  scraper minted (`[1-9]\d{8}`);
- none of those numbers appears at any other meet (a real number recurs
  across meets; a random one never does).

Anything else (a group containing a real-looking id that is seen elsewhere,
or a real id mixed with random ones) is left alone. The kept row is the one
with a session assignment, else the newest; its `member_id` is rewritten to
the `noid:` placeholder so future ingests match it. Re-running finds nothing
to do.

```sh
cd scrapers
# dry run first; prints each group it would collapse
DATABASE_URL=postgres://... PYTHONPATH=. python -m common.dedupe_idless_athletes --meet "2026 Ohio WSO Championships"
# apply for one meet, or for every meet in the athletes table
DATABASE_URL=postgres://... PYTHONPATH=. python -m common.dedupe_idless_athletes --meet "..." --apply
DATABASE_URL=postgres://... PYTHONPATH=. python -m common.dedupe_idless_athletes --all-meets --apply
```

All writes for a run happen in one transaction; the script refuses an empty
meet name. Take a backup before `--all-meets --apply` on production; it has
only been run against the local test database.

Tests: `scrapers/common/tests/test_dedupe_idless_athletes.py` (DB-backed) and
the `test_idless_athlete_*` cases in
`scrapers/common/tests/test_postgres_ingest.py`.
