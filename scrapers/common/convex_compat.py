from __future__ import annotations

from typing import Any

from common import postgres_writer as pg


class ConvexClient:
    """Small compatibility shim for copied scrapers.

    Existing scripts call Convex scraperIngestion actions. During the backend migration, this class
    keeps that call shape but writes directly to Postgres through DATABASE_URL.
    """

    def __init__(self, _url: str | None = None):
        self.url = _url

    def action(self, path: str, args: dict[str, Any]) -> dict[str, Any]:
        with pg.connect() as conn:
            result = self._dispatch(conn, path, args)
            conn.commit()
            return result

    def _dispatch(self, conn, path: str, args: dict[str, Any]) -> dict[str, Any]:
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
            return {"deleted": 0}

        raise NotImplementedError(f"Unsupported scraper action: {path}")
