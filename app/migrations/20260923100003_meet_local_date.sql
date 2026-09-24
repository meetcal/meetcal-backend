-- "Today" in a meet's own time zone, for date-window filters on `meets`.
--
-- `CURRENT_DATE` is the session (UTC) date, which runs up to a day ahead of a
-- US meet's local date every evening. `NOW() AT TIME ZONE time_zone` raises
-- `invalid_parameter_value` for a zone name Postgres does not know, and one
-- bad row must not fail the whole listing, so the fallback is the UTC date.
CREATE OR REPLACE FUNCTION meet_local_date(time_zone TEXT)
RETURNS DATE
LANGUAGE plpgsql
STABLE
AS $$
BEGIN
    RETURN COALESCE((NOW() AT TIME ZONE time_zone)::date, CURRENT_DATE);
EXCEPTION
    WHEN OTHERS THEN
        RETURN CURRENT_DATE;
END
$$;
