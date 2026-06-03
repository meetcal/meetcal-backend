CREATE TABLE IF NOT EXISTS intl_rankings (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    legacy_id BIGINT,
    meet TEXT,
    ranking DOUBLE PRECISION,
    name TEXT,
    weight_class TEXT,
    total DOUBLE PRECISION,
    percent_a DOUBLE PRECISION,
    gender TEXT,
    age_category TEXT
);

CREATE INDEX IF NOT EXISTS idx_intl_rankings_meet
    ON intl_rankings (meet);

CREATE INDEX IF NOT EXISTS idx_intl_rankings_gender_age
    ON intl_rankings (gender, age_category);

CREATE TABLE IF NOT EXISTS standards (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    age_category TEXT NOT NULL,
    gender TEXT NOT NULL,
    weight_class TEXT NOT NULL,
    standard_a DOUBLE PRECISION NOT NULL,
    standard_b DOUBLE PRECISION NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_standards_age_gender
    ON standards (age_category, gender);

CREATE TABLE IF NOT EXISTS records (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    record_type TEXT NOT NULL,
    age_category TEXT NOT NULL,
    gender TEXT NOT NULL,
    weight_class TEXT NOT NULL,
    snatch_record DOUBLE PRECISION,
    cj_record DOUBLE PRECISION,
    total_record DOUBLE PRECISION
);

CREATE INDEX IF NOT EXISTS idx_records_record_type
    ON records (record_type);

CREATE INDEX IF NOT EXISTS idx_records_type_age_gender
    ON records (record_type, age_category, gender);

CREATE TABLE IF NOT EXISTS wso_records (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,

    wso TEXT NOT NULL,
    age_category TEXT NOT NULL,
    gender TEXT NOT NULL,
    weight_class TEXT NOT NULL,
    snatch_record DOUBLE PRECISION,
    cj_record DOUBLE PRECISION,
    total_record DOUBLE PRECISION
);

CREATE INDEX IF NOT EXISTS idx_wso_records_wso
    ON wso_records (wso);

CREATE INDEX IF NOT EXISTS idx_wso_records_wso_age_gender
    ON wso_records (wso, age_category, gender);
