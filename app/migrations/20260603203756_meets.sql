CREATE TABLE IF NOT EXISTS meets (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    name TEXT NOT NULL,
    federation TEXT,

    start_date DATE NOT NULL,
    end_date DATE NOT NULL,

    status TEXT NOT NULL CHECK (
        status IN ('upcoming', 'ongoing', 'completed')
    ),

    time_zone TEXT NOT NULL,

    updated_at BIGINT NOT NULL,

    venue_name TEXT NOT NULL,
    venue_street TEXT NOT NULL,
    venue_city TEXT NOT NULL,
    venue_state TEXT NOT NULL,
    venue_zip TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_meets_name
    ON meets (name);

CREATE INDEX IF NOT EXISTS idx_meets_start_date
    ON meets (start_date DESC);

CREATE INDEX IF NOT EXISTS idx_meets_end_date
    ON meets (end_date DESC);

CREATE INDEX IF NOT EXISTS idx_meets_status
    ON meets (status);

CREATE INDEX IF NOT EXISTS idx_meets_status_start_date
    ON meets (status, start_date DESC);
