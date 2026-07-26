-- Referral rewards program (Phase 1).
--
-- Four tables:
--   referral_codes       one per user; the shareable invite code.
--   referrals            one attribution per referred user (UNIQUE), tracks the
--                        pending -> qualifying -> qualified/disqualified lifecycle.
--   subscription_events  raw RevenueCat webhook audit log, idempotent on event_id.
--   reward_ledger        ONE ROW PER 30-day reward unit (not a counter), so each
--                        reward can be claimed, delivered, retried, or reversed
--                        independently and idempotently.
--
-- RLS + grants live in the companion 20260722120001 migration.

CREATE TABLE IF NOT EXISTS referral_codes (
    id BIGSERIAL PRIMARY KEY,

    user_id TEXT NOT NULL UNIQUE,
    code TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_referral_codes_code ON referral_codes (code);

CREATE TABLE IF NOT EXISTS referrals (
    id BIGSERIAL PRIMARY KEY,

    referrer_user_id TEXT NOT NULL,
    -- One attribution ever: a referred user can be attributed to exactly one
    -- referrer, exactly once.
    referred_user_id TEXT NOT NULL UNIQUE,
    referral_code TEXT NOT NULL,

    signup_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    subscription_started_at TIMESTAMPTZ,

    -- pending    -> signed up via code, not yet paid
    -- qualifying -> first paid (NORMAL) period seen, inside the 30-day hold
    -- qualified  -> 30-day hold cleared with no disqualifying event
    -- disqualified -> refund/chargeback/fraud, or attribution voided
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'qualifying', 'qualified', 'disqualified')),
    qualified_at TIMESTAMPTZ,
    disqualification_reason TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Anti-abuse: no self-referral (defense in depth; also enforced in the app).
    CONSTRAINT referrals_no_self CHECK (referrer_user_id <> referred_user_id)
);

CREATE INDEX IF NOT EXISTS idx_referrals_referrer ON referrals (referrer_user_id);
CREATE INDEX IF NOT EXISTS idx_referrals_status ON referrals (status);
CREATE INDEX IF NOT EXISTS idx_referrals_referrer_status
    ON referrals (referrer_user_id, status);

CREATE TABLE IF NOT EXISTS subscription_events (
    id BIGSERIAL PRIMARY KEY,

    -- RevenueCat event id; UNIQUE gives us webhook idempotency.
    event_id TEXT NOT NULL UNIQUE,
    app_user_id TEXT,
    store TEXT,
    type TEXT,
    period_type TEXT,
    product_id TEXT,
    event JSONB NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_subscription_events_app_user
    ON subscription_events (app_user_id);
CREATE INDEX IF NOT EXISTS idx_subscription_events_type ON subscription_events (type);

CREATE TABLE IF NOT EXISTS reward_ledger (
    id BIGSERIAL PRIMARY KEY,

    user_id TEXT NOT NULL,

    -- earned     -> minted, unclaimed
    -- claimed    -> RESERVED (unused after the no-consume iOS redesign; the
    --               constraint is tightened to drop it in 20260722120003)
    -- delivering -> Android defer in flight
    -- delivered  -> subscription extended by 30 days
    -- reversed   -> clawed back (only ever from earned status)
    -- failed     -> RESERVED (unused; Android delivery errors roll back to earned)
    status TEXT NOT NULL DEFAULT 'earned'
        CHECK (status IN ('earned', 'claimed', 'delivering', 'delivered', 'reversed', 'failed')),

    -- Null until claim; then 'ios' | 'android'.
    platform TEXT CHECK (platform IN ('ios', 'android')),
    store_transaction_id TEXT,

    -- Deterministic mint/delivery key; UNIQUE prevents double-mint on concurrent
    -- qualification runs and double-extend on delivery retries.
    idempotency_key TEXT NOT NULL UNIQUE,

    earned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ,
    delivered_at TIMESTAMPTZ,
    reversed_at TIMESTAMPTZ,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_reward_ledger_user ON reward_ledger (user_id);
CREATE INDEX IF NOT EXISTS idx_reward_ledger_user_status ON reward_ledger (user_id, status);
