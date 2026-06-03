CREATE TABLE IF NOT EXISTS athletes (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    member_id TEXT NOT NULL,
    name TEXT NOT NULL,
    age DOUBLE PRECISION NOT NULL,
    club TEXT NOT NULL,
    wso TEXT,
    gender TEXT NOT NULL,
    weight_class TEXT NOT NULL,
    entry_total DOUBLE PRECISION NOT NULL,
    session_number DOUBLE PRECISION,
    session_platform TEXT,
    meet TEXT NOT NULL,
    adaptive BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_athletes_meet ON athletes (meet);
CREATE INDEX IF NOT EXISTS idx_athletes_meet_session
    ON athletes (meet, session_number, session_platform);
CREATE INDEX IF NOT EXISTS idx_athletes_adaptive ON athletes (adaptive);
CREATE INDEX IF NOT EXISTS idx_athletes_member_id ON athletes (member_id);
CREATE INDEX IF NOT EXISTS idx_athletes_club ON athletes (club);
CREATE INDEX IF NOT EXISTS idx_athletes_club_meet ON athletes (club, meet);

CREATE TABLE IF NOT EXISTS qualifying_totals (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    event_name TEXT NOT NULL,
    gender TEXT NOT NULL,
    age_category TEXT NOT NULL,
    weight_class TEXT NOT NULL,
    qualifying_total DOUBLE PRECISION NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_qualifying_totals_event ON qualifying_totals (event_name);
CREATE INDEX IF NOT EXISTS idx_qualifying_totals_event_gender_age
    ON qualifying_totals (event_name, gender, age_category);

CREATE TABLE IF NOT EXISTS session_schedule (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    date TEXT NOT NULL,
    session_id DOUBLE PRECISION NOT NULL,
    start_time TEXT NOT NULL,
    weigh_in_time TEXT NOT NULL,
    platform TEXT NOT NULL,
    weight_class TEXT NOT NULL,
    meet TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_session_schedule_meet ON session_schedule (meet);
CREATE INDEX IF NOT EXISTS idx_session_schedule_meet_session
    ON session_schedule (meet, session_id);
CREATE INDEX IF NOT EXISTS idx_session_schedule_date ON session_schedule (date);

CREATE TABLE IF NOT EXISTS saved_sessions (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    session_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    meet TEXT NOT NULL,
    session_number DOUBLE PRECISION NOT NULL,
    platform TEXT NOT NULL,
    weight_class TEXT,
    start_time TEXT,
    notes TEXT,
    athlete_names TEXT[],
    date TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_saved_sessions_user_id ON saved_sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_saved_sessions_session_user
    ON saved_sessions (session_id, user_id);
CREATE INDEX IF NOT EXISTS idx_saved_sessions_date ON saved_sessions (date);

CREATE TABLE IF NOT EXISTS user_preferences (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    user_id TEXT NOT NULL UNIQUE,
    auto_unsave_started_sessions BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_user_preferences_user_id ON user_preferences (user_id);
CREATE INDEX IF NOT EXISTS idx_user_preferences_auto_unsave
    ON user_preferences (auto_unsave_started_sessions);
