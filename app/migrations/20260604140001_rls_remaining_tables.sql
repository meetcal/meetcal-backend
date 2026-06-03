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

ALTER TABLE user_preferences ENABLE ROW LEVEL SECURITY;
ALTER TABLE user_preferences FORCE ROW LEVEL SECURITY;

GRANT SELECT ON athletes TO meetcal_api;
GRANT SELECT ON qualifying_totals TO meetcal_api;
GRANT SELECT ON session_schedule TO meetcal_api;

REVOKE ALL ON saved_sessions FROM meetcal_api;
REVOKE ALL ON user_preferences FROM meetcal_api;
