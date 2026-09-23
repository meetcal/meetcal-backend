-- The ingest writer now stores known platforms in canonical casing ("red" ->
-- "Red"), but rows written before that keep whatever casing the source had.
-- `athletes.session_platform` is joined to `session_schedule.platform` by
-- exact equality, so a partial re-ingest (athletes and schedule written by
-- different runs) would split one platform in two. Bring existing rows to the
-- same canonical spelling. Must match KNOWN_PLATFORMS in
-- scrapers/common/normalize.py. Idempotent: only rows that differ are touched.
SET LOCAL lock_timeout = '5s';

WITH known(platform) AS (
    VALUES ('Red'), ('White'), ('Blue'), ('Stars'), ('Stripes'), ('Rogue')
)
UPDATE session_schedule s
SET platform = k.platform
FROM known k
WHERE lower(btrim(s.platform)) = lower(k.platform)
  AND s.platform IS DISTINCT FROM k.platform;

WITH known(platform) AS (
    VALUES ('Red'), ('White'), ('Blue'), ('Stars'), ('Stripes'), ('Rogue')
)
UPDATE athletes a
SET session_platform = k.platform
FROM known k
WHERE lower(btrim(a.session_platform)) = lower(k.platform)
  AND a.session_platform IS DISTINCT FROM k.platform;
