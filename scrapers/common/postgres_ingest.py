#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from typing import Any

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
    if path == "scraperIngestion:deleteAthletesByMeet":
        deleted = conn.execute(
            "DELETE FROM athletes WHERE meet = %s RETURNING 1",
            (args.get("meet", ""),),
        ).rowcount
        return {"deleted": deleted}
    if path == "scraperIngestion:ingestSessionSchedule":
        return pg.upsert_session_schedule(conn, args)
    if path == "scraperIngestion:deleteSessionScheduleByMeet":
        deleted = conn.execute(
            "DELETE FROM session_schedule WHERE meet = %s RETURNING 1",
            (args.get("meet", ""),),
        ).rowcount
        return {"deleted": deleted}
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
        conn.execute("DELETE FROM intl_rankings")
        inserted = 0
        for row in args.get("rankings", []):
            pg.upsert_intl_ranking(conn, row)
            inserted += 1
        return {"inserted": inserted}
    if path == "scraperIngestion:deleteMissingIntlRankingGroups":
        return pg.delete_missing_intl_ranking_groups(conn, args.get("groups", []))

    raise NotImplementedError(f"Unsupported scraper action: {path}")


class IngestClient:
    def action(self, path: str, args: dict[str, Any]) -> dict[str, Any]:
        with pg.connect() as conn:
            result = dispatch(conn, path, args)
            conn.commit()
            return result


def main() -> int:
    if len(sys.argv) != 2:
        print("Usage: postgres_ingest.py <scraperIngestion:path>", file=sys.stderr)
        return 2

    path = sys.argv[1]
    payload = json.load(sys.stdin)
    rows = payload if isinstance(payload, list) else [payload]

    with pg.connect() as conn:
        results = [dispatch(conn, path, row) for row in rows]
        conn.commit()

    print(json.dumps(results if isinstance(payload, list) else results[0]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
