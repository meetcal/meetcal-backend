-- Mark meets completed once their end_date has passed in the meet's own time
-- zone, not the server's UTC date. `meet_local_date(time_zone)` (migration
-- 20260923100003_meet_local_date.sql) is the one spelling of that rule: it
-- falls back to the UTC date for a null, blank or unknown zone name instead
-- of failing the whole statement. Run by `run_scraper_job.sh
-- complete-ended-meets`; exercised by common/tests/test_complete_ended_meets.py.
WITH updated AS (
    UPDATE meets
    SET
        status = 'completed',
        updated_at = (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT
    WHERE status <> 'completed'
      AND end_date < meet_local_date(time_zone)
    RETURNING id
)
SELECT COUNT(*) FROM updated;
