-- Row-level security for the referral rewards tables.
--
-- Two access modes:
--   * USER context  — request.jwt.claim.sub is set by set_request_user() from the
--     verified Clerk JWT. User-facing reads (my code, my rewards, my progress as a
--     referrer) are scoped to the caller via app_current_user_id().
--   * BACKEND context — request.meetcal.backend = 'on' is set by set_backend_context()
--     for server-authoritative flows that are NOT tied to one end user: the
--     RevenueCat webhook, the daily qualification cron, and the redeem lookup that
--     must resolve someone else's code. Those flows enforce their own authorization
--     in application code, so RLS grants them broad access here.
--
-- Only our own API code can set either GUC (users never issue raw SQL), so the
-- backend marker is not user-forgeable.

CREATE OR REPLACE FUNCTION app_is_backend()
RETURNS BOOLEAN
LANGUAGE SQL
STABLE
AS $$
    SELECT current_setting('request.meetcal.backend', true) = 'on'
$$;

-- referral_codes: a user reads/creates their own code; the redeem lookup reads
-- another user's code under backend context.
ALTER TABLE referral_codes ENABLE ROW LEVEL SECURITY;
ALTER TABLE referral_codes FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS referral_codes_select ON referral_codes;
CREATE POLICY referral_codes_select ON referral_codes
    FOR SELECT
    USING (user_id = app_current_user_id() OR app_is_backend());

DROP POLICY IF EXISTS referral_codes_insert ON referral_codes;
CREATE POLICY referral_codes_insert ON referral_codes
    FOR INSERT
    WITH CHECK (user_id = app_current_user_id() OR app_is_backend());

DROP POLICY IF EXISTS referral_codes_update ON referral_codes;
CREATE POLICY referral_codes_update ON referral_codes
    FOR UPDATE
    USING (app_is_backend())
    WITH CHECK (app_is_backend());

-- referrals: a user may read rows where they are the REFERRER (progress counts
-- only; the app never selects referred_user_id back to a client). A user may
-- insert only a row where they are the REFERRED user (redeem). All status
-- transitions happen under backend context (webhook / cron).
ALTER TABLE referrals ENABLE ROW LEVEL SECURITY;
ALTER TABLE referrals FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS referrals_select ON referrals;
CREATE POLICY referrals_select ON referrals
    FOR SELECT
    USING (referrer_user_id = app_current_user_id() OR app_is_backend());

DROP POLICY IF EXISTS referrals_insert ON referrals;
CREATE POLICY referrals_insert ON referrals
    FOR INSERT
    WITH CHECK (referred_user_id = app_current_user_id() OR app_is_backend());

DROP POLICY IF EXISTS referrals_update ON referrals;
CREATE POLICY referrals_update ON referrals
    FOR UPDATE
    USING (app_is_backend())
    WITH CHECK (app_is_backend());

-- subscription_events: backend-only. No end user ever reads or writes this table.
ALTER TABLE subscription_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE subscription_events FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS subscription_events_backend ON subscription_events;
CREATE POLICY subscription_events_backend ON subscription_events
    FOR ALL
    USING (app_is_backend())
    WITH CHECK (app_is_backend());

-- reward_ledger: a user reads and claims (updates) their OWN rewards. Minting and
-- reversal happen under backend context.
ALTER TABLE reward_ledger ENABLE ROW LEVEL SECURITY;
ALTER TABLE reward_ledger FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS reward_ledger_select ON reward_ledger;
CREATE POLICY reward_ledger_select ON reward_ledger
    FOR SELECT
    USING (user_id = app_current_user_id() OR app_is_backend());

DROP POLICY IF EXISTS reward_ledger_insert ON reward_ledger;
CREATE POLICY reward_ledger_insert ON reward_ledger
    FOR INSERT
    WITH CHECK (app_is_backend());

DROP POLICY IF EXISTS reward_ledger_update ON reward_ledger;
CREATE POLICY reward_ledger_update ON reward_ledger
    FOR UPDATE
    USING (user_id = app_current_user_id() OR app_is_backend())
    WITH CHECK (user_id = app_current_user_id() OR app_is_backend());

GRANT SELECT, INSERT, UPDATE ON referral_codes TO meetcal_api;
GRANT SELECT, INSERT, UPDATE ON referrals TO meetcal_api;
GRANT SELECT, INSERT ON subscription_events TO meetcal_api;
GRANT SELECT, INSERT, UPDATE ON reward_ledger TO meetcal_api;

GRANT USAGE, SELECT ON SEQUENCE referral_codes_id_seq TO meetcal_api;
GRANT USAGE, SELECT ON SEQUENCE referrals_id_seq TO meetcal_api;
GRANT USAGE, SELECT ON SEQUENCE subscription_events_id_seq TO meetcal_api;
GRANT USAGE, SELECT ON SEQUENCE reward_ledger_id_seq TO meetcal_api;
