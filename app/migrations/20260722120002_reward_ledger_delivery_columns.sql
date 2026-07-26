-- Follow-up to the referral rewards tables:
--   * iOS delivery is confirmed by the RevenueCat webhook (a claim no longer
--     consumes the reward), so record when a signature was last handed out for
--     audit/debugging, and store the Android new-expiry in its own column
--     instead of mislabelling it as store_transaction_id.
--   * Tighten the reward_ledger UPDATE policy to backend context only: every
--     status transition (claim audit, delivery, reversal, mint) runs under
--     set_backend_context(); no user-context code path updates this table.

ALTER TABLE reward_ledger
    ADD COLUMN IF NOT EXISTS ios_offer_issued_at TIMESTAMPTZ;

ALTER TABLE reward_ledger
    ADD COLUMN IF NOT EXISTS delivered_expiry_ms BIGINT;

DROP POLICY IF EXISTS reward_ledger_update ON reward_ledger;
CREATE POLICY reward_ledger_update ON reward_ledger
    FOR UPDATE
    USING (app_is_backend())
    WITH CHECK (app_is_backend());
