CREATE INDEX IF NOT EXISTS idx_session_schedule_meet_session_platform
    ON session_schedule (meet, session_id, platform);
