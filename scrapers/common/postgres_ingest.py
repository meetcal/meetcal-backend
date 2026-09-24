#!/usr/bin/env python3
from __future__ import annotations

import json
import logging
import sys
from dataclasses import dataclass
from typing import Any, Iterable

from common import postgres_writer as pg


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


def main() -> int:
    if len(sys.argv) != 2:
        print("Usage: postgres_ingest.py <scraperIngestion:path>", file=sys.stderr)
        return 2

    path = sys.argv[1]
    payload = json.load(sys.stdin)
    rows = payload if isinstance(payload, list) else [payload]

    results = IngestClient().actions(path, rows)

    print(json.dumps(results if isinstance(payload, list) else results[0]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
