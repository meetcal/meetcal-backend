#!/usr/bin/env python3
"""Print meet status JSON for a name on stdin: {"name":"..."} -> {"status":"..."|null}."""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import postgres_writer as pg  # noqa: E402


def main() -> None:
    payload = json.load(sys.stdin)
    name = payload.get("name") or ""
    with pg.connect() as conn:
        row = conn.execute(
            "SELECT status FROM meets WHERE name = %s LIMIT 1", (name,)
        ).fetchone()
    print(json.dumps({"status": row["status"] if row else None}))


if __name__ == "__main__":
    main()
