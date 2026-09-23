-- The Python writer's upsert lookup (scrapers/common/postgres_writer.py)
-- matches `legacy_id IS NOT DISTINCT FROM %s AND legacy_id IS NOT NULL` as one
-- arm of its OR, which had no index and forced a sequential scan per row
-- whenever a legacy id was supplied. Partial: NULL legacy ids are never matched.
CREATE INDEX IF NOT EXISTS idx_lifting_results_legacy_id
    ON lifting_results (legacy_id)
    WHERE legacy_id IS NOT NULL;
