-- The API refuses to start when the database is missing a migration it was
-- built against (see `ensure_migrations_applied` in src/lib.rs), so the
-- least-privileged role it runs as must be able to read the ledger.
-- `_sqlx_migrations` predates the default privileges that cover later tables.
SET LOCAL lock_timeout = '5s';

DO $$
BEGIN
    IF to_regclass('public._sqlx_migrations') IS NOT NULL
       AND EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'meetcal_api') THEN
        GRANT SELECT ON public._sqlx_migrations TO meetcal_api;
    END IF;
END
$$;
