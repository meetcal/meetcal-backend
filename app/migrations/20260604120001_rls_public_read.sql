DO $$
BEGIN
    CREATE ROLE meetcal_api WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
EXCEPTION
    WHEN duplicate_object THEN NULL;
END
$$;

GRANT CONNECT ON DATABASE meetcal TO meetcal_api;
GRANT USAGE ON SCHEMA public TO meetcal_api;

ALTER TABLE meets ENABLE ROW LEVEL SECURITY;
ALTER TABLE meets FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS public_read ON meets;
CREATE POLICY public_read ON meets
    FOR SELECT
    USING (true);

ALTER TABLE intl_rankings ENABLE ROW LEVEL SECURITY;
ALTER TABLE intl_rankings FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS public_read ON intl_rankings;
CREATE POLICY public_read ON intl_rankings
    FOR SELECT
    USING (true);

ALTER TABLE standards ENABLE ROW LEVEL SECURITY;
ALTER TABLE standards FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS public_read ON standards;
CREATE POLICY public_read ON standards
    FOR SELECT
    USING (true);

ALTER TABLE records ENABLE ROW LEVEL SECURITY;
ALTER TABLE records FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS public_read ON records;
CREATE POLICY public_read ON records
    FOR SELECT
    USING (true);

ALTER TABLE wso_records ENABLE ROW LEVEL SECURITY;
ALTER TABLE wso_records FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS public_read ON wso_records;
CREATE POLICY public_read ON wso_records
    FOR SELECT
    USING (true);

GRANT SELECT ON meets TO meetcal_api;
GRANT SELECT ON intl_rankings TO meetcal_api;
GRANT SELECT ON standards TO meetcal_api;
GRANT SELECT ON records TO meetcal_api;
GRANT SELECT ON wso_records TO meetcal_api;

ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT SELECT ON TABLES TO meetcal_api;
