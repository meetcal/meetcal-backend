-- Functional indexes supporting case- and whitespace-insensitive athlete name
-- matching. The expression must match `normalize_name` in app/src/common/names.rs
-- and NORMALIZED_NAME_SQL: lower(btrim(regexp_replace(name, '\s+', ' ', 'g'))).

CREATE INDEX IF NOT EXISTS idx_lifting_results_name_normalized
    ON lifting_results (lower(btrim(regexp_replace(name, '\s+', ' ', 'g'))));

CREATE INDEX IF NOT EXISTS idx_lifting_results_name_normalized_date
    ON lifting_results (lower(btrim(regexp_replace(name, '\s+', ' ', 'g'))), date);

CREATE INDEX IF NOT EXISTS idx_athletes_name_normalized
    ON athletes (lower(btrim(regexp_replace(name, '\s+', ' ', 'g'))));
