-- Mark meets completed once their end_date has passed in the meet's own time
-- zone, not the server's. `NOW() AT TIME ZONE <zone>` raises on an unknown
-- zone name, so only a name present in pg_timezone_names is used; a null,
-- blank or unknown time_zone falls back to UTC rather than failing the whole
-- statement. Run by `run_scraper_job.sh complete-ended-meets`; exercised by
-- common/tests/test_complete_ended_meets.py.
WITH updated AS (
    UPDATE meets
    SET
        status = 'completed',
        updated_at = (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT
    WHERE status <> 'completed'
      AND end_date < (
          NOW() AT TIME ZONE (
              CASE
                  WHEN EXISTS (
                      SELECT 1 FROM pg_timezone_names tz WHERE tz.name = meets.time_zone
                  ) THEN meets.time_zone
                  ELSE 'UTC'
              END
          )
      )::date
    RETURNING id
)
SELECT COUNT(*) FROM updated;
