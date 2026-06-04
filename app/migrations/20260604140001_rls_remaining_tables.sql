ALTER TABLE athletes ENABLE ROW LEVEL SECURITY;
ALTER TABLE athletes FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS public_read ON athletes;
CREATE POLICY public_read ON athletes
    FOR SELECT
    USING (true);

ALTER TABLE qualifying_totals ENABLE ROW LEVEL SECURITY;
ALTER TABLE qualifying_totals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS public_read ON qualifying_totals;
CREATE POLICY public_read ON qualifying_totals
    FOR SELECT
    USING (true);

ALTER TABLE session_schedule ENABLE ROW LEVEL SECURITY;
ALTER TABLE session_schedule FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS public_read ON session_schedule;
CREATE POLICY public_read ON session_schedule
    FOR SELECT
    USING (true);

ALTER TABLE saved_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE saved_sessions FORCE ROW LEVEL SECURITY;
CREATE OR REPLACE FUNCTION app_current_user_id()
RETURNS TEXT
LANGUAGE SQL
STABLE
AS $$
    SELECT COALESCE(
        NULLIF(current_setting('request.jwt.claim.sub', true), ''),
        NULLIF(NULLIF(current_setting('request.jwt.claims', true), '')::jsonb ->> 'sub', ''),
        NULLIF(NULLIF(current_setting('request.jwt.claims', true), '')::jsonb ->> 'user_id', '')
    )
$$;

DROP POLICY IF EXISTS saved_sessions_select_own ON saved_sessions;
CREATE POLICY saved_sessions_select_own ON saved_sessions
    FOR SELECT
    USING (user_id = app_current_user_id());

DROP POLICY IF EXISTS saved_sessions_insert_own ON saved_sessions;
CREATE POLICY saved_sessions_insert_own ON saved_sessions
    FOR INSERT
    WITH CHECK (user_id = app_current_user_id());

DROP POLICY IF EXISTS saved_sessions_update_own ON saved_sessions;
CREATE POLICY saved_sessions_update_own ON saved_sessions
    FOR UPDATE
    USING (user_id = app_current_user_id())
    WITH CHECK (user_id = app_current_user_id());

DROP POLICY IF EXISTS saved_sessions_delete_own ON saved_sessions;
CREATE POLICY saved_sessions_delete_own ON saved_sessions
    FOR DELETE
    USING (user_id = app_current_user_id());

ALTER TABLE user_preferences ENABLE ROW LEVEL SECURITY;
ALTER TABLE user_preferences FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS user_preferences_select_own ON user_preferences;
CREATE POLICY user_preferences_select_own ON user_preferences
    FOR SELECT
    USING (user_id = app_current_user_id());

DROP POLICY IF EXISTS user_preferences_insert_own ON user_preferences;
CREATE POLICY user_preferences_insert_own ON user_preferences
    FOR INSERT
    WITH CHECK (user_id = app_current_user_id());

DROP POLICY IF EXISTS user_preferences_update_own ON user_preferences;
CREATE POLICY user_preferences_update_own ON user_preferences
    FOR UPDATE
    USING (user_id = app_current_user_id())
    WITH CHECK (user_id = app_current_user_id());

GRANT SELECT ON athletes TO meetcal_api;
GRANT SELECT ON qualifying_totals TO meetcal_api;
GRANT SELECT ON session_schedule TO meetcal_api;
GRANT SELECT, INSERT, UPDATE, DELETE ON saved_sessions TO meetcal_api;
GRANT SELECT, INSERT, UPDATE ON user_preferences TO meetcal_api;
GRANT USAGE, SELECT ON SEQUENCE saved_sessions_id_seq TO meetcal_api;
GRANT USAGE, SELECT ON SEQUENCE user_preferences_id_seq TO meetcal_api;
