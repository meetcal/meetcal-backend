-- Round-2 review follow-ups:
--   * subscription_events gains `cancel_reason` (RevenueCat sends this at the
--     same level as `type`; a customer-support refund is CANCELLATION +
--     cancel_reason CUSTOMER_SUPPORT) and `purchased_at` (from purchased_at_ms /
--     event_timestamp_ms). Both feed the disqualification predicate and the
--     first-paid-after-attribution (H2a) promotion rule.
--   * reward_ledger status CHECK: drop the never-written 'claimed' and 'failed'
--     values so the constraint reflects reality after the no-consume iOS
--     redesign (earned -> delivering -> delivered, or reversed).

ALTER TABLE subscription_events
    ADD COLUMN IF NOT EXISTS cancel_reason TEXT;

ALTER TABLE subscription_events
    ADD COLUMN IF NOT EXISTS purchased_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_subscription_events_user_purchased
    ON subscription_events (app_user_id, purchased_at);

-- Reflect the real lifecycle: 'claimed' and 'failed' are never written.
--   earned      -> minted, unclaimed
--   delivering  -> Android defer in flight (transient, within one tx)
--   delivered   -> subscription extended (Android) / promo month applied (iOS)
--   reversed    -> clawed back (only ever from earned)
ALTER TABLE reward_ledger DROP CONSTRAINT IF EXISTS reward_ledger_status_check;
ALTER TABLE reward_ledger
    ADD CONSTRAINT reward_ledger_status_check
    CHECK (status IN ('earned', 'delivering', 'delivered', 'reversed'));
