"""Ingest a staged bundle to Postgres.

The pipeline writes athletes, schedule, and optional meet metadata through
the shared ``common.postgres_writer`` helpers. Replace semantics: by default
existing athletes + schedule rows for the meet are deleted before insert, so
a re-run is a clean replacement rather than an accumulation.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

SCRAPERS_DIR = Path(__file__).resolve().parents[2]
if str(SCRAPERS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRAPERS_DIR))


def _ingest_postgres(
    athletes: List[Dict[str, Any]],
    schedule: List[Dict[str, Any]],
    meet: Optional[Dict[str, Any]],
    meet_name: str,
    replace: bool,
) -> Dict[str, Any]:
    from common import postgres_writer as pg

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


def ingest_bundle(
    athletes: List[Dict[str, Any]],
    schedule: List[Dict[str, Any]],
    meet: Optional[Dict[str, Any]],
    meet_name: str,
    replace: bool = True,
    on_target_done: Optional[Callable[[str, Dict[str, Any]], None]] = None,
) -> Dict[str, Any]:
    """Ingest a staged bundle to Postgres. Returns stats.

    ``on_target_done(target, stats)`` is invoked after the write finishes, so
    callers can post a Slack confirmation.
    """
    stats = _ingest_postgres(athletes, schedule, meet, meet_name, replace)
    if on_target_done is not None:
        on_target_done("postgres", stats)
    return {"target": "postgres", "replace": replace, "postgres": stats}
