"""Bring a test database to the real schema before DB-backed tests run.

CI's Postgres service starts empty, and unittest discovery runs the test files
in alphabetical order, so every DB-backed test class applies the migrations
itself (idempotently) rather than relying on another file having run first.
"""
from __future__ import annotations

import hashlib
from pathlib import Path

import psycopg

MIGRATIONS_DIR = Path(__file__).resolve().parents[3] / "app" / "migrations"

# Same shape sqlx creates, so `sqlx migrate run` and these tests agree on what
# has been applied to a database.
SQLX_MIGRATIONS_DDL = """
CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
    success BOOLEAN NOT NULL,
    checksum BYTEA NOT NULL,
    execution_time BIGINT NOT NULL
)
"""


def apply_migrations(url: str) -> list[int]:
    """Apply every unapplied ``app/migrations/*.sql`` in order; returns the
    versions applied by this call. Idempotent: versions already recorded in
    ``_sqlx_migrations`` (by sqlx or by an earlier run) are skipped."""
    paths = sorted(MIGRATIONS_DIR.glob("*.sql"))
    if not paths:
        raise RuntimeError(f"no migrations found under {MIGRATIONS_DIR}")
    with psycopg.connect(url, autocommit=True) as conn:
        conn.execute(SQLX_MIGRATIONS_DDL)
        applied = {row[0] for row in conn.execute("SELECT version FROM _sqlx_migrations").fetchall()}
    newly_applied: list[int] = []
    for path in paths:
        version_text, description = path.stem.split("_", 1)
        version = int(version_text)
        if version in applied:
            continue
        sql = path.read_text(encoding="utf-8")
        with psycopg.connect(url) as conn:
            conn.execute(sql)
            conn.execute(
                """
                INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
                VALUES (%s, %s, TRUE, %s, 0)
                """,
                (version, description, hashlib.sha384(sql.encode("utf-8")).digest()),
            )
            conn.commit()
        newly_applied.append(version)
    return newly_applied
