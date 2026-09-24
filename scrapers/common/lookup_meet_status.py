#!/usr/bin/env python3
"""Print meet statuses as JSON for the meet name(s) on stdin.

One name:   {"name": "..."}            -> {"status": "..." | null}
Many names: {"names": ["...", ...]}    -> {"statuses": {"<name>": "..." | null, ...}}

The many-names form answers a whole sync run with one process and one query.
Stdin is held to the same ``MAX_STDIN_BYTES`` / ``MAX_STDIN_ROWS`` caps as
``postgres_ingest.py``; past either cap it exits 3 without querying.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import postgres_writer as pg  # noqa: E402
from common.postgres_ingest import (  # noqa: E402
    MAX_STDIN_ROWS,
    PayloadTooLarge,
    read_payload,
)


def lookup_statuses(conn, names: list[str]) -> dict[str, str | None]:
    """Status per requested name; ``None`` when no meet has that exact name."""
    if not names:
        return {}
    # One query; each requested name is matched exactly (as the old per-name
    # ``WHERE name = %s LIMIT 1`` did) and answered by its position.
    rows = conn.execute(
        """
        SELECT requested.idx, meet.status
        FROM unnest(%s::text[]) WITH ORDINALITY AS requested(name, idx)
        LEFT JOIN LATERAL (
            SELECT status FROM meets WHERE meets.name = requested.name LIMIT 1
        ) AS meet ON true
        """,
        (names,),
    ).fetchall()
    by_index = {row["idx"]: row["status"] for row in rows}
    return {name: by_index.get(index) for index, name in enumerate(names, start=1)}


def main() -> int:
    try:
        payload = read_payload(sys.stdin.buffer)
    except PayloadTooLarge as error:
        print(f"lookup_meet_status.py: {error}", file=sys.stderr)
        return 3
    if not isinstance(payload, dict):
        print('lookup_meet_status.py: expected {"name": ...} or {"names": [...]}', file=sys.stderr)
        return 2

    if "names" in payload:
        names = payload["names"]
        if not isinstance(names, list) or not all(isinstance(n, str) for n in names):
            print("lookup_meet_status.py: names must be a list of strings", file=sys.stderr)
            return 2
        if len(names) > MAX_STDIN_ROWS:
            print(
                f"lookup_meet_status.py: {len(names)} names; the limit is {MAX_STDIN_ROWS}",
                file=sys.stderr,
            )
            return 3
        with pg.connect() as conn:
            statuses = lookup_statuses(conn, names)
        print(json.dumps({"statuses": statuses}))
        return 0

    name = payload.get("name") or ""
    with pg.connect() as conn:
        status = lookup_statuses(conn, [name])[name]
    print(json.dumps({"status": status}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
