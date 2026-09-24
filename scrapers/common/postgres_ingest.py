#!/usr/bin/env python3
from __future__ import annotations

import json
import logging
import sys
from dataclasses import dataclass
from typing import Any, Iterable

from common import postgres_writer as pg

# Ceilings on what ``main`` accepts on stdin. The largest real payload is one
# meet's entry list (a national championship is ~2,000 athletes at a few hundred
# bytes each, well under 1 MiB), so these leave an order of magnitude of
# headroom while refusing a runaway or corrupted producer before it is parsed
# into memory or written row by row.
MAX_STDIN_BYTES = 16 * 1024 * 1024
MAX_STDIN_ROWS = 20_000


def dispatch(conn, path: str, args: dict[str, Any]) -> dict[str, Any]:
    if path == "scraperIngestion:ingestLiftingResult":
        return pg.upsert_lifting_result(conn, args)
    if path == "scraperIngestion:ingestRecord":
        return pg.upsert_record(conn, args)
    if path == "scraperIngestion:replaceIWFRecords":
        return pg.replace_records(conn, "IWF", args.get("records", []))
    if path == "scraperIngestion:ingestQualifyingTotal":
        return pg.upsert_qualifying_total(conn, args)
    if path == "scraperIngestion:ingestStandard":
        return pg.upsert_standard(conn, args)
    if path == "scraperIngestion:ingestAthlete":
        return pg.upsert_athlete(conn, args)
    if path == "scraperIngestion:ingestEntryAthlete":
        return pg.upsert_athlete(conn, args, preserve_assigned_session=True)
    if path == "scraperIngestion:deleteAthletesByMeet":
        return {"deleted": pg.delete_athletes_by_meet(conn, args.get("meet", ""))}
    if path == "scraperIngestion:ingestSessionSchedule":
        return pg.upsert_session_schedule(conn, args)
    if path == "scraperIngestion:deleteSessionScheduleByMeet":
        return {
            "deleted": pg.delete_session_schedule_by_meet(conn, args.get("meet", ""))
        }
    if path == "scraperIngestion:ingestWSORecord":
        return pg.upsert_wso_record(conn, args)
    if path == "scraperIngestion:replaceWSORecordSet":
        return pg.replace_wso_records(conn, args.get("wso", ""), args.get("records", []))
    if path == "scraperIngestion:ingestMeet":
        return pg.upsert_meet(conn, args)
    if path == "scraperIngestion:ingestIntlRanking":
        return pg.upsert_intl_ranking(conn, args)
    if path == "scraperIngestion:replaceIntlRankingsForGroup":
        return pg.replace_intl_rankings_group(conn, args)
    if path == "scraperIngestion:replaceAllIntlRankings":
        return pg.replace_all_intl_rankings(conn, args.get("rankings", []))
    if path == "scraperIngestion:deleteMissingIntlRankingGroups":
        return pg.delete_missing_intl_ranking_groups(conn, args.get("groups", []))

    raise NotImplementedError(f"Unsupported scraper action: {path}")


@dataclass(frozen=True)
class RowFailure:
    """A row ``actions(..., skip_errors=True)`` could not write; the rest were."""

    index: int
    error: Exception


class IngestClient:
    """Thin dispatch wrapper: one connection + one transaction per call.

    Prefer ``actions`` for anything that loops over records. ``action`` opens
    a connection and commits per row, which is the wrong shape for a scraper
    writing hundreds of records and leaves a partial write on failure.
    """

    def action(self, path: str, args: dict[str, Any]) -> dict[str, Any]:
        return self.actions(path, [args])[0]

    def actions(self, path: str, rows: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
        """Dispatch every row on one connection in one transaction.

        All-or-nothing: a failing row raises and rolls back every earlier row
        in the batch (``pg.connect`` rolls back when the block exits with an
        exception), so a scraper never leaves half a record set behind. Use
        this for replace-style writes. Returns one result per row, in order.
        """
        rows = list(rows)
        if not rows:
            return []
        with pg.connect() as conn:
            results = [dispatch(conn, path, row) for row in rows]
            conn.commit()
            return results

    def actions_skipping_errors(
        self, path: str, rows: Iterable[dict[str, Any]]
    ) -> list[dict[str, Any] | RowFailure]:
        """Dispatch every row on one connection, skipping rows that fail.

        For independent upserts (one lifter's result, one record) where a bad
        row should be logged and skipped rather than cost the rest of the set:
        each row runs in its own savepoint inside one transaction, so a failure
        undoes only that row. Returns, per row in order, the dispatch result or
        a ``RowFailure``.
        """
        rows = list(rows)
        if not rows:
            return []
        results: list[dict[str, Any] | RowFailure] = []
        with pg.connect() as conn:
            with conn.transaction():
                for index, row in enumerate(rows):
                    try:
                        with conn.transaction():
                            results.append(dispatch(conn, path, row))
                    except Exception as error:  # noqa: BLE001 - reported per row
                        logging.error("Ingest %s row %s failed; skipped: %s", path, index, error)
                        results.append(RowFailure(index, error))
        return results


class PayloadTooLarge(ValueError):
    """Stdin exceeded ``MAX_STDIN_BYTES`` or held more than ``MAX_STDIN_ROWS`` rows."""


def read_payload(
    stream,
    max_bytes: int = MAX_STDIN_BYTES,
    max_rows: int = MAX_STDIN_ROWS,
) -> Any:
    """Read and parse one JSON payload (an object or a list of row objects).

    Reads at most ``max_bytes + 1`` bytes so an oversized payload is refused
    without being buffered whole. A list longer than ``max_rows`` is refused
    before any row is written.
    """
    data = stream.read(max_bytes + 1)
    if len(data) > max_bytes:
        raise PayloadTooLarge(f"stdin payload exceeds the {max_bytes}-byte limit")
    payload = json.loads(data)
    if isinstance(payload, list) and len(payload) > max_rows:
        raise PayloadTooLarge(
            f"stdin payload has {len(payload)} rows; the limit is {max_rows}"
        )
    return payload


USAGE = "Usage: postgres_ingest.py [--skip-errors] <scraperIngestion:path>"
SKIP_ERRORS_FLAG = "--skip-errors"


def _row_failure_json(failure: RowFailure) -> dict[str, Any]:
    """How ``--skip-errors`` reports a row it could not write.

    ``rowError`` is a key no dispatch result uses (``skipped`` is taken: an
    entry athlete whose session is already assigned reports ``skipped``).
    """
    return {"rowError": str(failure.error), "index": failure.index}


def main(argv: list[str] | None = None) -> int:
    """Read one JSON payload from stdin and dispatch it on one connection.

    ``postgres_ingest.py <path>`` writes every row in one transaction
    (``IngestClient.actions``): any failing row rolls the whole payload back
    and the process exits non-zero. ``postgres_ingest.py --skip-errors <path>``
    gives each row its own savepoint (``IngestClient.actions_skipping_errors``)
    and exits 0 once the transaction commits; a row that failed is reported in
    its place in the output as ``{"rowError": "...", "index": i}``.
    Exit 3 means stdin was over ``MAX_STDIN_BYTES`` / ``MAX_STDIN_ROWS`` and
    nothing was written.
    """
    args = sys.argv[1:] if argv is None else argv
    skip_errors = False
    if len(args) == 2 and args[0] == SKIP_ERRORS_FLAG:
        skip_errors = True
        path = args[1]
    elif len(args) == 1 and not args[0].startswith("-"):
        path = args[0]
    else:
        print(USAGE, file=sys.stderr)
        return 2

    try:
        payload = read_payload(sys.stdin.buffer)
    except PayloadTooLarge as error:
        print(f"postgres_ingest.py {path}: {error}; nothing written", file=sys.stderr)
        return 3
    rows = payload if isinstance(payload, list) else [payload]

    if skip_errors:
        results = [
            _row_failure_json(result) if isinstance(result, RowFailure) else result
            for result in IngestClient().actions_skipping_errors(path, rows)
        ]
    else:
        results = IngestClient().actions(path, rows)

    print(json.dumps(results if isinstance(payload, list) else results[0]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
