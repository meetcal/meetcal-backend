-- The Slack /meets-add-pdf, /meets-add-map and /meets-remove-* commands are
-- the one Postgres write the API makes. Production connects as meetcal_api,
-- whose FORCE ROW LEVEL SECURITY + SELECT-only policy on meets made that
-- UPDATE fail (no grant) or match zero rows (no UPDATE policy). Grant exactly
-- the two venue-map columns and add the matching row policy; every other
-- column of meets stays read-only for the role.
GRANT UPDATE (venue_map_pdf_url, venue_map_apple_url) ON meets TO meetcal_api;

DROP POLICY IF EXISTS venue_map_update ON meets;
CREATE POLICY venue_map_update ON meets
    FOR UPDATE
    TO meetcal_api
    USING (true)
    WITH CHECK (true);
