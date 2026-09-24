"""Stand-in for ``$POSTGRES_INGEST_PYTHON`` in test_meet_sync_batching.py.

The meet-sync scripts run ``$POSTGRES_INGEST_PYTHON <script> [args...]`` with
a JSON payload on stdin, where ``<script>`` is ``common/postgres_ingest.py`` or
``common/lookup_meet_status.py``. This records every call as one JSON line in
``$FAKE_INGEST_LOG`` and answers without a database. Meet names steer it:

- ``BAD`` in a name: that row fails (``--skip-errors`` reports ``rowError``).
- ``EXISTING`` in a name: the upsert updates rather than inserts.
- ``CRASH`` in a name: the whole ingest call exits 1, as if Postgres were down.
- ``COMPLETED`` in a name: the status lookup answers ``completed``.
- ``$FAKE_LOOKUP_FAIL=1``: every status lookup exits 1.

The real stdin caps apply, read from ``postgres_ingest.py`` itself so the fake
cannot drift from them: past either cap the call exits 3, like the real CLI.
"""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path

INGEST_SOURCE = Path(__file__).resolve().parents[1] / "postgres_ingest.py"


def _cap(name: str) -> int:
    match = re.search(rf"^{name} = (.+)$", INGEST_SOURCE.read_text(encoding="utf-8"), re.M)
    if match is None:
        raise SystemExit(f"fake_ingest: {name} not found in {INGEST_SOURCE}")
    value = 1
    for factor in match.group(1).split("*"):  # "16 * 1024 * 1024" or "20_000"
        value *= int(factor.strip())
    return value


def main() -> int:
    script = Path(sys.argv[1]).name
    args = sys.argv[2:]
    data = sys.stdin.buffer.read()
    max_bytes, max_rows = _cap("MAX_STDIN_BYTES"), _cap("MAX_STDIN_ROWS")
    payload = json.loads(data) if len(data) <= max_bytes else None
    with open(os.environ["FAKE_INGEST_LOG"], "a", encoding="utf-8") as log:
        log.write(json.dumps({"script": script, "args": args, "bytes": len(data), "payload": payload}) + "\n")

    if payload is None:
        print(f"{script}: stdin payload exceeds the {max_bytes}-byte limit", file=sys.stderr)
        return 3

    if script == "lookup_meet_status.py":
        if os.environ.get("FAKE_LOOKUP_FAIL") == "1":
            print("lookup: connection refused", file=sys.stderr)
            return 1
        names = payload["names"]
        if len(names) > max_rows:
            return 3
        statuses = {n: ("completed" if "COMPLETED" in n else None) for n in names}
        print(json.dumps({"statuses": statuses}))
        return 0

    if script != "postgres_ingest.py":
        print(f"fake_ingest: unexpected script {script}", file=sys.stderr)
        return 2
    rows = payload if isinstance(payload, list) else [payload]
    if len(rows) > max_rows:
        print(f"{script}: stdin payload has {len(rows)} rows; the limit is {max_rows}", file=sys.stderr)
        return 3
    if any("CRASH" in row.get("name", "") for row in rows):
        print("connection refused", file=sys.stderr)
        return 1
    skip_errors = args[:1] == ["--skip-errors"]
    results = []
    for index, row in enumerate(rows):
        name = row.get("name", "")
        if "BAD" in name:
            if not skip_errors:
                print(f"row {index} failed: bad meet", file=sys.stderr)
                return 1
            results.append({"rowError": "bad meet", "index": index})
        else:
            results.append({"id": str(index), "wasInsert": "EXISTING" not in name, "wasChanged": True})
    print(json.dumps(results if isinstance(payload, list) else results[0]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
