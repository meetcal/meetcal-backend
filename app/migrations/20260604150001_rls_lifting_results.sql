ALTER TABLE lifting_results ENABLE ROW LEVEL SECURITY;
ALTER TABLE lifting_results FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS public_read ON lifting_results;
CREATE POLICY public_read ON lifting_results
    FOR SELECT
    USING (true);

GRANT SELECT ON lifting_results TO meetcal_api;
