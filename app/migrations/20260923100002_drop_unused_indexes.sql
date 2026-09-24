-- Indexes that serve no query in app/src or scrapers/ (verified by grep over
-- every SQL statement in both trees on 2026-09-23). Each one costs write
-- amplification on the ingest path for nothing.
--
-- idx_lifting_results_name_normalized: a strict prefix of
--   idx_lifting_results_name_normalized_date, which serves the same equality
--   lookups.
-- idx_athletes_adaptive: no query filters athletes by adaptive.
-- idx_lifting_results_adaptive: /data/adaptive filters on (adaptive, federation),
--   which idx_lifting_results_adaptive_federation covers.
-- idx_saved_sessions_date: saved sessions are only ever looked up by user_id.
-- idx_session_schedule_date: session_schedule is only ever looked up by meet.
-- idx_user_preferences_auto_unsave: preferences are only ever looked up by user_id.
DROP INDEX IF EXISTS idx_lifting_results_name_normalized;
DROP INDEX IF EXISTS idx_athletes_adaptive;
DROP INDEX IF EXISTS idx_lifting_results_adaptive;
DROP INDEX IF EXISTS idx_saved_sessions_date;
DROP INDEX IF EXISTS idx_session_schedule_date;
DROP INDEX IF EXISTS idx_user_preferences_auto_unsave;
