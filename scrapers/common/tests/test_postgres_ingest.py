"""Postgres-backed tests for scraper ingest dispatch and ranking prune.

Requires DATABASE_URL. Skips when unset so meet-automation unit tests still
run without a database.
"""

from __future__ import annotations

import os
import unittest
import uuid

try:
    import psycopg
    from psycopg.rows import dict_row

    from common import postgres_writer as pg
    from common.postgres_ingest import dispatch
except ImportError:  # pragma: no cover - optional local dep
    psycopg = None
    dict_row = None
    pg = None
    dispatch = None

INTL_RANKINGS_DDL = """
CREATE TABLE IF NOT EXISTS intl_rankings (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,
    legacy_id BIGINT,
    meet TEXT,
    ranking DOUBLE PRECISION,
    name TEXT,
    weight_class TEXT,
    total DOUBLE PRECISION,
    percent_a DOUBLE PRECISION,
    gender TEXT,
    age_category TEXT
)
"""

SESSION_SCHEDULE_DDL = """
CREATE TABLE IF NOT EXISTS session_schedule (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,
    date TEXT NOT NULL,
    session_id DOUBLE PRECISION NOT NULL,
    start_time TEXT NOT NULL,
    weigh_in_time TEXT NOT NULL,
    platform TEXT NOT NULL,
    weight_class TEXT NOT NULL,
    meet TEXT NOT NULL
)
"""

ATHLETES_DDL = """
CREATE TABLE IF NOT EXISTS athletes (
    id BIGSERIAL PRIMARY KEY,
    convex_id TEXT NOT NULL UNIQUE,
    member_id TEXT NOT NULL,
    name TEXT NOT NULL,
    age DOUBLE PRECISION NOT NULL,
    club TEXT NOT NULL,
    wso TEXT,
    gender TEXT NOT NULL,
    weight_class TEXT NOT NULL,
    entry_total DOUBLE PRECISION NOT NULL,
    session_number DOUBLE PRECISION,
    session_platform TEXT,
    meet TEXT NOT NULL,
    adaptive BOOLEAN NOT NULL DEFAULT FALSE
)
"""


def _ranking(meet: str, gender: str, age_category: str, name: str, ranking: int) -> dict:
    return {
        "meet": meet,
        "gender": gender,
        "ageCategory": age_category,
        "ranking": ranking,
        "name": name,
        "weightClass": "71",
        "total": 200,
    }


def _schedule(meet: str, session_id: int, platform: str) -> dict:
    return {
        "meet": meet,
        "date": "2026-06-20",
        "sessionId": session_id,
        "startTime": "09:00:00",
        "weighInTime": "07:00:00",
        "platform": platform,
        "weightClass": "71",
    }


@unittest.skipUnless(
    os.getenv("DATABASE_URL") and psycopg is not None,
    "DATABASE_URL and psycopg are required",
)
class PostgresIngestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        url = os.environ["DATABASE_URL"]
        with psycopg.connect(url, autocommit=True) as conn:
            conn.execute(INTL_RANKINGS_DDL)
            conn.execute(SESSION_SCHEDULE_DDL)
            conn.execute(ATHLETES_DDL)

    def setUp(self) -> None:
        self.token = uuid.uuid4().hex[:8]
        self.conn = psycopg.connect(os.environ["DATABASE_URL"], row_factory=dict_row)

    def tearDown(self) -> None:
        self.conn.rollback()
        self.conn.close()

    def _existing_intl_groups(self) -> list[dict[str, str]]:
        rows = self.conn.execute(
            "SELECT DISTINCT meet, gender, age_category FROM intl_rankings"
        ).fetchall()
        return [
            {
                "meet": row["meet"],
                "gender": row["gender"],
                "ageCategory": row["age_category"],
            }
            for row in rows
            if row["meet"] and row["gender"] and row["age_category"]
        ]

    def test_dispatch_unknown_path(self) -> None:
        with self.assertRaises(NotImplementedError):
            dispatch(self.conn, "scraperIngestion:notARealAction", {})

    def test_dispatch_session_schedule_round_trip(self) -> None:
        meet = f"__test_sched_{self.token}__"
        inserted = dispatch(self.conn, "scraperIngestion:ingestSessionSchedule", _schedule(meet, 1, "Red"))
        self.assertTrue(inserted["wasInsert"])
        deleted = dispatch(
            self.conn, "scraperIngestion:deleteSessionScheduleByMeet", {"meet": meet}
        )
        self.assertEqual(deleted["deleted"], 1)
        remaining = self.conn.execute(
            "SELECT COUNT(*) AS c FROM session_schedule WHERE meet = %s", (meet,)
        ).fetchone()
        self.assertEqual(remaining["c"], 0)

    def test_replace_schedule_in_one_transaction(self) -> None:
        meet = f"__test_replace_{self.token}__"
        dispatch(self.conn, "scraperIngestion:ingestSessionSchedule", _schedule(meet, 1, "Red"))
        dispatch(self.conn, "scraperIngestion:ingestSessionSchedule", _schedule(meet, 2, "White"))
        deleted = dispatch(
            self.conn, "scraperIngestion:deleteSessionScheduleByMeet", {"meet": meet}
        )
        self.assertEqual(deleted["deleted"], 2)
        dispatch(self.conn, "scraperIngestion:ingestSessionSchedule", _schedule(meet, 3, "Blue"))
        rows = self.conn.execute(
            "SELECT session_id, platform FROM session_schedule WHERE meet = %s",
            (meet,),
        ).fetchall()
        self.assertEqual([(row["session_id"], row["platform"]) for row in rows], [(3.0, "Blue")])

    def test_delete_missing_intl_ranking_groups(self) -> None:
        keep_meet = f"__test_keep_{self.token}__"
        drop_meet = f"__test_drop_{self.token}__"
        preserved = self._existing_intl_groups()
        pg.upsert_intl_ranking(self.conn, _ranking(keep_meet, "Women", "Senior", "Keep", 1))
        pg.upsert_intl_ranking(self.conn, _ranking(drop_meet, "Men", "Senior", "Drop", 1))

        result = pg.delete_missing_intl_ranking_groups(
            self.conn,
            [
                *preserved,
                {"meet": keep_meet, "gender": "Women", "ageCategory": "Senior"},
            ],
        )
        self.assertEqual(result["deleted"], 1)
        self.assertEqual(len(result["deletedGroups"]), 1)
        self.assertEqual(result["deletedGroups"][0]["meet"], drop_meet)

        kept = self.conn.execute(
            "SELECT meet FROM intl_rankings WHERE meet IN (%s, %s) ORDER BY meet",
            (keep_meet, drop_meet),
        ).fetchall()
        self.assertEqual([row["meet"] for row in kept], [keep_meet])

    def test_delete_missing_intl_ranking_groups_empty_is_noop(self) -> None:
        meet = f"__test_noop_{self.token}__"
        pg.upsert_intl_ranking(self.conn, _ranking(meet, "Women", "Senior", "Keep", 1))
        result = dispatch(
            self.conn, "scraperIngestion:deleteMissingIntlRankingGroups", {"groups": []}
        )
        self.assertEqual(result["deleted"], 0)
        remaining = self.conn.execute(
            "SELECT COUNT(*) AS c FROM intl_rankings WHERE meet = %s", (meet,)
        ).fetchone()
        self.assertEqual(remaining["c"], 1)

    def test_dispatch_delete_missing_intl_ranking_groups(self) -> None:
        keep_meet = f"__test_disp_keep_{self.token}__"
        drop_meet = f"__test_disp_drop_{self.token}__"
        preserved = self._existing_intl_groups()
        pg.upsert_intl_ranking(self.conn, _ranking(keep_meet, "Men", "U20", "Keep", 1))
        pg.upsert_intl_ranking(self.conn, _ranking(drop_meet, "Men", "U20", "Drop", 1))
        result = dispatch(
            self.conn,
            "scraperIngestion:deleteMissingIntlRankingGroups",
            {
                "groups": [
                    *preserved,
                    {"meet": keep_meet, "gender": "Men", "ageCategory": "U20"},
                ]
            },
        )
        self.assertEqual(result["deleted"], 1)
        remaining = self.conn.execute(
            "SELECT meet FROM intl_rankings WHERE meet IN (%s, %s)",
            (keep_meet, drop_meet),
        ).fetchall()
        self.assertEqual([row["meet"] for row in remaining], [keep_meet])

    def test_delete_athletes_requires_meet(self) -> None:
        with self.assertRaisesRegex(ValueError, "meet is required"):
            dispatch(self.conn, "scraperIngestion:deleteAthletesByMeet", {"meet": ""})
        with self.assertRaisesRegex(ValueError, "meet is required"):
            dispatch(self.conn, "scraperIngestion:deleteSessionScheduleByMeet", {})

    def test_replace_all_intl_rankings_rejects_empty_payload(self) -> None:
        with self.assertRaisesRegex(ValueError, "empty payload"):
            dispatch(self.conn, "scraperIngestion:replaceAllIntlRankings", {"rankings": []})

    def test_replace_intl_rankings_group_requires_identity(self) -> None:
        with self.assertRaisesRegex(ValueError, "meet is required"):
            dispatch(
                self.conn,
                "scraperIngestion:replaceIntlRankingsForGroup",
                {"meet": " ", "gender": "Women", "ageCategory": "Senior", "rankings": []},
            )

    def test_replace_intl_rankings_group_exact_set(self) -> None:
        meet = f"__test_group_{self.token}__"
        pg.upsert_intl_ranking(self.conn, _ranking(meet, "Women", "Senior", "Keep", 1))
        pg.upsert_intl_ranking(self.conn, _ranking(meet, "Women", "Senior", "Drop", 2))
        result = dispatch(
            self.conn,
            "scraperIngestion:replaceIntlRankingsForGroup",
            {
                "meet": meet,
                "gender": "Women",
                "ageCategory": "Senior",
                "rankings": [
                    _ranking(meet, "Women", "Senior", "Keep", 1),
                    _ranking(meet, "Women", "Senior", "New", 3),
                ],
            },
        )
        self.assertEqual(result["inserted"], 1)
        self.assertEqual(result["deleted"], 1)
        names = {
            row["name"]
            for row in self.conn.execute(
                "SELECT name FROM intl_rankings WHERE meet = %s", (meet,)
            ).fetchall()
        }
        self.assertEqual(names, {"Keep", "New"})

    def test_entry_athlete_ingest_keeps_assigned_sessions(self) -> None:
        meet = f"__test_entry_gate_{self.token}__"

        def athlete(name: str, member_id: str, **extra: object) -> dict:
            row = {
                "memberId": member_id,
                "name": name,
                "age": 21,
                "club": "Original",
                "gender": "Female",
                "weightClass": "59",
                "entryTotal": 180,
                "meet": meet,
            }
            row.update(extra)
            return row

        pg.upsert_athlete(
            self.conn,
            athlete("Session Both", "1", sessionNumber=4, sessionPlatform="Red"),
        )
        pg.upsert_athlete(self.conn, athlete("Session Number", "2", sessionNumber=0))
        pg.upsert_athlete(self.conn, athlete("Session Platform", "3", sessionPlatform="Blue"))
        pg.upsert_athlete(self.conn, athlete("Open", "4"))
        pg.upsert_athlete(self.conn, athlete("Blank Platform", "6", sessionPlatform="   "))
        pg.upsert_athlete(
            self.conn,
            athlete("Start List", "7", sessionNumber=8, sessionPlatform="Gold"),
        )

        with self.assertLogs("common.postgres_writer", level="WARNING") as logs:
            both = dispatch(
                self.conn,
                "scraperIngestion:ingestEntryAthlete",
                athlete("Session Both", "1", club="Changed", entryTotal=999),
            )
            number = dispatch(
                self.conn,
                "scraperIngestion:ingestEntryAthlete",
                athlete("Session Number", "2", club="Changed", entryTotal=999),
            )
            platform = dispatch(
                self.conn,
                "scraperIngestion:ingestEntryAthlete",
                athlete("Session Platform", "3", club="Changed", entryTotal=999),
            )

        opened = dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            athlete("Open", "4", club="Updated", entryTotal=210),
        )
        blank = dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            athlete("Blank Platform", "6", club="Updated", entryTotal=205),
        )
        inserted = dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            athlete("New Lifter", "5", club="Fresh", entryTotal=100),
        )
        overwritten = dispatch(
            self.conn,
            "scraperIngestion:ingestAthlete",
            athlete(
                "Start List",
                "7",
                club="Replaced",
                entryTotal=111,
                sessionNumber=9,
                sessionPlatform="Silver",
            ),
        )

        rows = {
            row["name"]: row
            for row in self.conn.execute(
                """
                SELECT name, club, entry_total, session_number, session_platform
                FROM athletes
                WHERE meet = %s
                """,
                (meet,),
            ).fetchall()
        }

        self.assertTrue(both["skipped"])
        self.assertEqual(both["skipReason"], "session already set")
        self.assertFalse(both["wasChanged"])
        self.assertTrue(number["skipped"])
        self.assertTrue(platform["skipped"])
        self.assertEqual(rows["Session Both"]["club"], "Original")
        self.assertEqual(float(rows["Session Both"]["entry_total"]), 180.0)
        self.assertEqual(float(rows["Session Both"]["session_number"]), 4.0)
        self.assertEqual(rows["Session Both"]["session_platform"], "Red")
        self.assertEqual(rows["Session Number"]["club"], "Original")
        self.assertEqual(float(rows["Session Number"]["session_number"]), 0.0)
        self.assertIsNone(rows["Session Number"]["session_platform"])
        self.assertEqual(rows["Session Platform"]["club"], "Original")
        self.assertIsNone(rows["Session Platform"]["session_number"])
        self.assertEqual(rows["Session Platform"]["session_platform"], "Blue")
        self.assertTrue(any("session already set" in line for line in logs.output))

        self.assertFalse(opened.get("skipped", False))
        self.assertTrue(opened["wasChanged"])
        self.assertEqual(rows["Open"]["club"], "Updated")
        self.assertEqual(float(rows["Open"]["entry_total"]), 210.0)
        self.assertIsNone(rows["Open"]["session_number"])
        self.assertIsNone(rows["Open"]["session_platform"])

        self.assertTrue(blank["wasChanged"])
        self.assertEqual(rows["Blank Platform"]["club"], "Updated")
        self.assertEqual(float(rows["Blank Platform"]["entry_total"]), 205.0)

        self.assertTrue(inserted["wasInsert"])
        self.assertEqual(rows["New Lifter"]["club"], "Fresh")
        self.assertIsNone(rows["New Lifter"]["session_number"])

        self.assertTrue(overwritten["wasChanged"])
        self.assertFalse(overwritten.get("skipped", False))
        self.assertEqual(rows["Start List"]["club"], "Replaced")
        self.assertEqual(float(rows["Start List"]["session_number"]), 9.0)
        self.assertEqual(rows["Start List"]["session_platform"], "Silver")

    def test_ingest_bundle_refuses_empty_meet_name(self) -> None:
        from usaw.meet_automation import ingest

        with self.assertRaisesRegex(ValueError, "meet_name is required"):
            ingest.ingest_bundle([], [], None, "  ", replace=True)

    def test_ingest_replace_rolls_back_on_write_failure(self) -> None:
        from unittest.mock import patch

        from usaw.meet_automation import ingest

        meet = f"__test_rb_{self.token}__"
        athlete = {
            "memberId": "1",
            "name": "Keep Me",
            "age": 24,
            "club": "Test",
            "gender": "Female",
            "weightClass": "71",
            "entryTotal": 200,
            "meet": meet,
        }
        pg.upsert_athlete(self.conn, athlete)
        self.conn.commit()
        try:
            with patch.object(pg, "upsert_athlete", side_effect=RuntimeError("boom")):
                with self.assertRaises(RuntimeError):
                    ingest.ingest_bundle([athlete], [], None, meet, replace=True)
            remaining = self.conn.execute(
                "SELECT COUNT(*) AS c FROM athletes WHERE meet = %s", (meet,)
            ).fetchone()
            self.assertEqual(remaining["c"], 1)
        finally:
            self.conn.execute("DELETE FROM athletes WHERE meet = %s", (meet,))
            self.conn.commit()
