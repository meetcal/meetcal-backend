"""Dual-write ingest of a staged bundle to Postgres and Convex.

For now the pipeline writes to BOTH backends (Convex is being phased out).
Targets are selectable so tests / dry environments can write only Postgres.

Postgres writes go in-process through the shared ``common.postgres_writer``
upsert helpers (the same path the other scrapers use via ``postgres_ingest``).
Convex writes go over HTTP to ``{CONVEX_URL}/api/action`` with the
``SCRAPER_SECRET``, mirroring the existing scrapers.

Replace semantics: by default existing athletes + schedule rows for the meet
are deleted before insert, so a re-run is a clean replacement rather than an
accumulation.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional

# Make ``common`` importable regardless of cwd.
SCRAPERS_DIR = Path(__file__).resolve().parents[2]
if str(SCRAPERS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRAPERS_DIR))

TARGET_POSTGRES = "postgres"
TARGET_CONVEX = "convex"
TARGET_BOTH = "both"

ATHLETE_INGEST_PATH = "scraperIngestion:ingestAthlete"
ATHLETE_DELETE_PATH = "scraperIngestion:deleteAthletesByMeet"
SCHEDULE_INGEST_PATH = "scraperIngestion:ingestSessionSchedule"
SCHEDULE_DELETE_PATH = "scraperIngestion:deleteSessionScheduleByMeet"
MEET_INGEST_PATH = "scraperIngestion:ingestMeet"
REQUEST_TIMEOUT_SECONDS = 45


def _resolve_targets(target: str) -> List[str]:
    if target == TARGET_BOTH:
        out = []
        if os.getenv("DATABASE_URL"):
            out.append(TARGET_POSTGRES)
        if os.getenv("CONVEX_URL") or os.getenv("EXPO_PUBLIC_CONVEX_URL"):
            out.append(TARGET_CONVEX)
        if not out:
            raise RuntimeError(
                "target=both but neither DATABASE_URL nor CONVEX_URL is set"
            )
        return out
    return [target]


# --------------------------------------------------------------------------
# Postgres
# --------------------------------------------------------------------------
def _ingest_postgres(
    athletes: List[Dict[str, Any]],
    schedule: List[Dict[str, Any]],
    meet: Optional[Dict[str, Any]],
    meet_name: str,
    replace: bool,
) -> Dict[str, Any]:
    from common import postgres_writer as pg  # lazy: only needs psycopg

    stats = {"athletes": {"inserted": 0, "updated": 0, "unchanged": 0},
             "schedule": {"inserted": 0, "updated": 0, "unchanged": 0},
             "meet": None,
             "deleted_athletes": 0,
             "deleted_schedule": 0}

    with pg.connect() as conn:
        if replace:
            stats["deleted_athletes"] = conn.execute(
                "DELETE FROM athletes WHERE meet = %s", (meet_name,)
            ).rowcount
            stats["deleted_schedule"] = conn.execute(
                "DELETE FROM session_schedule WHERE meet = %s", (meet_name,)
            ).rowcount

        if meet:
            res = pg.upsert_meet(conn, {**meet, "name": meet.get("name", meet_name)})
            stats["meet"] = res

        for row in athletes:
            res = pg.upsert_athlete(conn, {**row, "meet": meet_name})
            _tally(stats["athletes"], res)

        for row in schedule:
            res = pg.upsert_session_schedule(conn, {**row, "meet": meet_name})
            _tally(stats["schedule"], res)

        conn.commit()

    return stats


def _tally(bucket: Dict[str, int], res: Dict[str, Any]) -> None:
    if res.get("wasInsert"):
        bucket["inserted"] += 1
    elif res.get("wasChanged"):
        bucket["updated"] += 1
    else:
        bucket["unchanged"] += 1


# --------------------------------------------------------------------------
# Convex
# --------------------------------------------------------------------------
def _convex_action(endpoint: str, path: str, args: Dict[str, Any]) -> Dict[str, Any]:
    import requests  # lazy

    response = requests.post(
        endpoint, json={"path": path, "args": args}, timeout=REQUEST_TIMEOUT_SECONDS
    )
    response.raise_for_status()
    data = response.json()
    return data.get("value", {}) if isinstance(data, dict) else {}


def _ingest_convex(
    athletes: List[Dict[str, Any]],
    schedule: List[Dict[str, Any]],
    meet: Optional[Dict[str, Any]],
    meet_name: str,
    replace: bool,
) -> Dict[str, Any]:
    convex_url = os.getenv("CONVEX_URL") or os.getenv("EXPO_PUBLIC_CONVEX_URL")
    scraper_secret = os.getenv("SCRAPER_SECRET")
    if not convex_url or not scraper_secret:
        raise RuntimeError("CONVEX_URL and SCRAPER_SECRET are required for convex ingest")

    endpoint = f"{convex_url.rstrip('/')}/api/action"
    stats: Dict[str, Any] = {"athletes": 0, "schedule": 0, "failed": 0, "meet": None}

    if replace:
        _convex_action(endpoint, ATHLETE_DELETE_PATH, {"scraperSecret": scraper_secret, "meet": meet_name})
        _convex_action(endpoint, SCHEDULE_DELETE_PATH, {"scraperSecret": scraper_secret, "meet": meet_name})

    if meet:
        stats["meet"] = _convex_action(
            endpoint, MEET_INGEST_PATH, {"scraperSecret": scraper_secret, **meet, "name": meet.get("name", meet_name)}
        )

    for row in athletes:
        try:
            _convex_action(endpoint, ATHLETE_INGEST_PATH, {"scraperSecret": scraper_secret, **row, "meet": meet_name})
            stats["athletes"] += 1
        except Exception as exc:  # noqa: BLE001
            stats["failed"] += 1
            print(f"convex athlete error {row.get('name')}: {exc}", file=sys.stderr)

    for row in schedule:
        try:
            _convex_action(endpoint, SCHEDULE_INGEST_PATH, {"scraperSecret": scraper_secret, **row, "meet": meet_name})
            stats["schedule"] += 1
        except Exception as exc:  # noqa: BLE001
            stats["failed"] += 1
            print(f"convex schedule error {row.get('sessionId')}: {exc}", file=sys.stderr)

    return stats


# --------------------------------------------------------------------------
# Public entry point
# --------------------------------------------------------------------------
def ingest_bundle(
    athletes: List[Dict[str, Any]],
    schedule: List[Dict[str, Any]],
    meet: Optional[Dict[str, Any]],
    meet_name: str,
    target: str = TARGET_BOTH,
    replace: bool = True,
) -> Dict[str, Any]:
    """Ingest a staged bundle. Returns per-target stats."""
    results: Dict[str, Any] = {"target": target, "replace": replace}
    for resolved in _resolve_targets(target):
        if resolved == TARGET_POSTGRES:
            results[TARGET_POSTGRES] = _ingest_postgres(athletes, schedule, meet, meet_name, replace)
        elif resolved == TARGET_CONVEX:
            results[TARGET_CONVEX] = _ingest_convex(athletes, schedule, meet, meet_name, replace)
        else:
            raise ValueError(f"Unknown ingest target: {resolved}")
    return results
