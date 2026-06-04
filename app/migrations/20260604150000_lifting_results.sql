CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE IF NOT EXISTS lifting_results (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    legacy_id BIGINT,
    event_id TEXT NOT NULL,
    meet TEXT NOT NULL,
    date TEXT NOT NULL,
    name TEXT NOT NULL,
    age TEXT,
    body_weight DOUBLE PRECISION,
    snatch1 DOUBLE PRECISION,
    snatch2 DOUBLE PRECISION,
    snatch3 DOUBLE PRECISION,
    snatch_best DOUBLE PRECISION,
    cj1 DOUBLE PRECISION,
    cj2 DOUBLE PRECISION,
    cj3 DOUBLE PRECISION,
    cj_best DOUBLE PRECISION,
    total DOUBLE PRECISION,
    adaptive BOOLEAN NOT NULL DEFAULT FALSE,
    federation TEXT
);

CREATE INDEX IF NOT EXISTS idx_lifting_results_name
    ON lifting_results (name);

CREATE INDEX IF NOT EXISTS idx_lifting_results_name_trgm
    ON lifting_results USING GIN (name gin_trgm_ops);

CREATE INDEX IF NOT EXISTS idx_lifting_results_meet
    ON lifting_results (meet);

CREATE INDEX IF NOT EXISTS idx_lifting_results_date
    ON lifting_results (date);

CREATE INDEX IF NOT EXISTS idx_lifting_results_federation
    ON lifting_results (federation);

CREATE INDEX IF NOT EXISTS idx_lifting_results_adaptive
    ON lifting_results (adaptive);

CREATE INDEX IF NOT EXISTS idx_lifting_results_name_date
    ON lifting_results (name, date);

CREATE INDEX IF NOT EXISTS idx_lifting_results_event_name
    ON lifting_results (event_id, name);

CREATE INDEX IF NOT EXISTS idx_lifting_results_federation_age
    ON lifting_results (federation, age);

CREATE INDEX IF NOT EXISTS idx_lifting_results_adaptive_federation
    ON lifting_results (adaptive, federation);

CREATE INDEX IF NOT EXISTS idx_lifting_results_meet_name
    ON lifting_results (meet, name);
