#!/usr/bin/env python3
from __future__ import annotations

import json
import sys

from common import postgres_writer as pg
from common.convex_compat import ConvexClient


def main() -> int:
    if len(sys.argv) != 2:
        print("Usage: postgres_ingest.py <scraperIngestion:path>", file=sys.stderr)
        return 2

    path = sys.argv[1]
    payload = json.load(sys.stdin)
    rows = payload if isinstance(payload, list) else [payload]
    client = ConvexClient(None)

    with pg.connect() as conn:
        results = [client._dispatch(conn, path, row) for row in rows]
        conn.commit()

    print(json.dumps(results if isinstance(payload, list) else results[0]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
