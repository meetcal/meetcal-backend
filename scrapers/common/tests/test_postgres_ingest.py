"""Postgres-backed tests for scraper ingest dispatch and ranking prune.

Requires DATABASE_URL. Skips when unset so meet-automation unit tests still
run without a database.

The schema comes from the real ``app/migrations/*.sql`` files, applied in
filename order and recorded in ``_sqlx_migrations`` the way sqlx does, so a
migration that drifts from what the writer expects fails these tests instead
of a hand-written copy of the DDL hiding it.
"""

from __future__ import annotations

import os
import unittest
import uuid

try:
    import psycopg
    from psycopg.rows import dict_row

    from common import postgres_writer as pg
    from common.postgres_ingest import IngestClient, RowFailure, dispatch
    from common.tests.db_schema import MIGRATIONS_DIR, apply_migrations
except ImportError:  # pragma: no cover - optional local dep
    psycopg = None
    dict_row = None
    pg = None
    IngestClient = None
    dispatch = None

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
        apply_migrations(os.environ["DATABASE_URL"])

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
        logged = "\n".join(logs.output)
        self.assertIn("session already set", logged)
        self.assertIn("convex_id=", logged)
        self.assertNotIn("Session Both", logged)
        self.assertNotIn("member_id", logged)

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

    def test_entry_conflict_keeps_session_when_lookup_misses(self) -> None:
        meet = f"__test_entry_conflict_{self.token}__"

        def athlete(**extra: object) -> dict:
            row = {
                "memberId": "42",
                "name": "Race Lifter",
                "age": 21,
                "club": "Original",
                "gender": "Female",
                "weightClass": "59",
                "entryTotal": 180,
                "meet": meet,
                "sessionNumber": 4,
                "sessionPlatform": "Red",
            }
            row.update(extra)
            return row

        pg.upsert_athlete(self.conn, athlete())
        original_execute = self.conn.execute

        def execute(query, params=None):
            statement = " ".join(query.split()).upper()
            if (
                statement.startswith("SELECT")
                and "FROM ATHLETES" in statement
                and "FOR UPDATE" in statement
            ):
                original_execute(query, params)

                class Empty:
                    def fetchone(self):
                        return None

                return Empty()
            return original_execute(query, params)

        self.conn.execute = execute
        try:
            with self.assertLogs("common.postgres_writer", level="WARNING") as logs:
                pg.upsert_athlete(
                    self.conn,
                    athlete(club="Changed", entryTotal=999, sessionNumber=None, sessionPlatform=None),
                    preserve_assigned_session=True,
                )
        finally:
            self.conn.execute = original_execute

        row = self.conn.execute(
            """
            SELECT id, convex_id, club, entry_total, session_number, session_platform, name, member_id
            FROM athletes
            WHERE meet = %s
            """,
            (meet,),
        ).fetchone()
        self.assertEqual(row["club"], "Changed")
        self.assertEqual(float(row["entry_total"]), 999.0)
        self.assertEqual(float(row["session_number"]), 4.0)
        self.assertEqual(row["session_platform"], "Red")
        logged = "\n".join(logs.output)
        self.assertIn("kept existing session", logged)
        self.assertIn(f"id={row['id']}", logged)
        self.assertIn(f"convex_id={row['convex_id']}", logged)
        self.assertIn(f"meet={meet}", logged)
        self.assertNotIn(row["name"], logged)
        self.assertNotIn("member_id", logged)

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

    def test_migrations_are_applied_and_idempotent(self) -> None:
        # setUpClass already applied them; a second pass finds nothing to do
        # and every migration file is recorded.
        self.assertEqual(apply_migrations(os.environ["DATABASE_URL"]), [])
        recorded = {
            row["version"]
            for row in self.conn.execute("SELECT version FROM _sqlx_migrations").fetchall()
        }
        expected = {int(path.stem.split("_", 1)[0]) for path in MIGRATIONS_DIR.glob("*.sql")}
        self.assertTrue(expected)
        self.assertTrue(expected <= recorded, expected - recorded)

    def test_idless_athlete_ingests_update_one_row(self) -> None:
        meet = f"__test_idless_{self.token}__"

        def athlete(name: str, member_id: str, **extra: object) -> dict:
            row = {
                "memberId": member_id,
                "name": name,
                "age": 30,
                "club": "Club",
                "gender": "Female",
                "weightClass": "64",
                "entryTotal": 150,
                "meet": meet,
            }
            row.update(extra)
            return row

        first_run = dispatch(
            self.conn, "scraperIngestion:ingestEntryAthlete", athlete("Jane Doe", "")
        )
        # Nightly re-scrape: placeholder id, different name casing/whitespace.
        second_run = dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            athlete("jane  DOE", "noid:jane-doe", club="Moved", entryTotal=160),
        )
        # A real member id with the same name is a different athlete.
        real = dispatch(
            self.conn, "scraperIngestion:ingestEntryAthlete", athlete("Jane Doe", "123456")
        )
        rows = self.conn.execute(
            "SELECT member_id, name, club, entry_total FROM athletes WHERE meet = %s ORDER BY id",
            (meet,),
        ).fetchall()

        self.assertTrue(first_run["wasInsert"])
        self.assertFalse(second_run["wasInsert"])
        self.assertTrue(second_run["wasChanged"])
        self.assertEqual(second_run["id"], first_run["id"])
        self.assertTrue(real["wasInsert"])
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["member_id"], "noid:jane-doe")
        self.assertEqual(rows[0]["club"], "Moved")
        self.assertEqual(float(rows[0]["entry_total"]), 160.0)
        self.assertEqual(rows[1]["member_id"], "123456")

    def test_placeholder_ingest_adopts_a_pre_placeholder_random_id(self) -> None:
        meet = f"__test_random_id_{self.token}__"
        other_meet = f"__test_random_id_other_{self.token}__"
        row = {
            "age": 30,
            "club": "Club",
            "gender": "Female",
            "weightClass": "64",
            "entryTotal": 150,
        }
        # Scraped once by the old scraper: a random nine-digit id.
        old = dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            {**row, "memberId": "482913377", "name": "Jane Doe", "meet": meet},
        )
        # A real nine-digit membership number recurs at other meets and must
        # not be taken for a random one.
        dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            {**row, "memberId": "123456789", "name": "Sam Lifter", "meet": meet},
        )
        dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            {**row, "memberId": "123456789", "name": "Sam Lifter", "meet": other_meet},
        )

        jane = dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            {**row, "memberId": "noid:jane-doe", "name": "Jane Doe", "meet": meet},
        )
        sam = dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            {**row, "memberId": "noid:sam-lifter", "name": "Sam Lifter", "meet": meet},
        )

        self.assertFalse(jane["wasInsert"])
        self.assertEqual(jane["id"], old["id"])
        self.assertTrue(sam["wasInsert"])
        rows = self.conn.execute(
            "SELECT member_id FROM athletes WHERE meet = %s ORDER BY member_id",
            (meet,),
        ).fetchall()
        # Jane's row was adopted and keeps its number (it may be real);
        # Sam's recurring number is a real one, so he got his own row.
        self.assertEqual(
            [r["member_id"] for r in rows],
            ["123456789", "482913377", "noid:sam-lifter"],
        )

    def test_idless_same_name_different_gender_stay_separate(self) -> None:
        meet = f"__test_idless_gender_{self.token}__"
        base = {
            "memberId": "",
            "name": "Alex Lifter",
            "age": 30,
            "club": "Club",
            "weightClass": "71",
            "entryTotal": 200,
            "meet": meet,
        }
        woman = dispatch(self.conn, "scraperIngestion:ingestEntryAthlete", {**base, "gender": "Female"})
        man = dispatch(self.conn, "scraperIngestion:ingestEntryAthlete", {**base, "gender": "Male"})
        again = dispatch(
            self.conn,
            "scraperIngestion:ingestEntryAthlete",
            {**base, "gender": "Female", "club": "Moved"},
        )

        self.assertTrue(woman["wasInsert"])
        self.assertTrue(man["wasInsert"])
        self.assertNotEqual(woman["id"], man["id"])
        self.assertEqual(again["id"], woman["id"])

    def test_idless_entry_keeps_assigned_session(self) -> None:
        meet = f"__test_idless_session_{self.token}__"
        base = {
            "memberId": "",
            "name": "Sam Lifter",
            "age": 30,
            "club": "Club",
            "gender": "Male",
            "weightClass": "89",
            "entryTotal": 250,
            "meet": meet,
        }
        pg.upsert_athlete(self.conn, {**base, "sessionNumber": 3, "sessionPlatform": "red"})
        with self.assertLogs("common.postgres_writer", level="WARNING"):
            result = dispatch(
                self.conn,
                "scraperIngestion:ingestEntryAthlete",
                {**base, "memberId": "noid:sam-lifter", "club": "Changed"},
            )
        row = self.conn.execute(
            "SELECT club, session_number, session_platform FROM athletes WHERE meet = %s",
            (meet,),
        ).fetchone()
        self.assertTrue(result["skipped"])
        self.assertEqual(row["club"], "Club")
        self.assertEqual(float(row["session_number"]), 3.0)
        # Platform casing is canonicalised at ingest.
        self.assertEqual(row["session_platform"], "Red")

    def test_session_schedule_normalises_platform_and_times(self) -> None:
        meet = f"__test_sched_norm_{self.token}__"
        row = _schedule(meet, 1, "red")
        row["startTime"] = "14:30:00"
        row["weighInTime"] = "not a time"
        dispatch(self.conn, "scraperIngestion:ingestSessionSchedule", row)
        # Same session again with the canonical casing: same row, not a second one.
        again = dispatch(self.conn, "scraperIngestion:ingestSessionSchedule", _schedule(meet, 1, "Red"))
        stored = self.conn.execute(
            "SELECT platform, start_time, weigh_in_time FROM session_schedule WHERE meet = %s",
            (meet,),
        ).fetchall()
        self.assertFalse(again["wasInsert"])
        self.assertEqual(len(stored), 1)
        self.assertEqual(stored[0]["platform"], "Red")
        self.assertEqual(stored[0]["start_time"], "9:00 AM")
        self.assertEqual(stored[0]["weigh_in_time"], "7:00 AM")

    def test_lifting_result_lookup_precedence(self) -> None:
        event = f"__test_evt_{self.token}__"
        meet = f"__test_lr_{self.token}__"
        legacy = int(self.token[:6], 16) + 10_000_000_000

        def result(**extra: object) -> dict:
            row = {"eventId": event, "meet": meet, "date": "2026-06-20", "name": "Ada Lovelace", "total": 200}
            row.update(extra)
            return row

        # Three distinct rows: A has an explicit convex_id, B a legacy_id, and C
        # (explicit convex_id too, so a payload's derived id cannot match it)
        # is reachable only through its natural key.
        by_convex = pg.upsert_lifting_result(self.conn, result(convexId=f"lr_{self.token}_a", name="Ada A"))
        by_legacy = pg.upsert_lifting_result(self.conn, result(legacyId=legacy, name="Ada B"))
        by_natural = pg.upsert_lifting_result(self.conn, result(convexId=f"lr_{self.token}_c"))
        self.assertEqual(len({by_convex["id"], by_legacy["id"], by_natural["id"]}), 3)

        # Natural key only: the row with no other identity.
        wins_natural = pg.upsert_lifting_result(self.conn, result(total=203))
        self.assertEqual(wins_natural["id"], by_natural["id"])
        self.assertFalse(wins_natural["wasInsert"])
        # legacy_id beats the natural key: this payload matches B (legacy) and
        # C (event/meet/name) at once.
        wins_legacy = pg.upsert_lifting_result(self.conn, result(legacyId=legacy, total=202))
        self.assertEqual(wins_legacy["id"], by_legacy["id"])
        # convex_id beats legacy_id: this payload matches A (convex) and B (legacy).
        wins_convex = pg.upsert_lifting_result(
            self.conn, result(convexId=f"lr_{self.token}_a", legacyId=legacy, name="Ada A", total=201)
        )
        self.assertEqual(wins_convex["id"], by_convex["id"])

        totals = {
            str(row["id"]): float(row["total"])
            for row in self.conn.execute(
                "SELECT id, total FROM lifting_results WHERE meet = %s", (meet,)
            ).fetchall()
        }
        self.assertEqual(
            totals,
            {by_convex["id"]: 201.0, by_legacy["id"]: 202.0, by_natural["id"]: 203.0},
        )


@unittest.skipUnless(
    os.getenv("DATABASE_URL") and psycopg is not None,
    "DATABASE_URL and psycopg are required",
)
class IngestClientTests(unittest.TestCase):
    """`IngestClient` commits, so these clean up after themselves."""

    @classmethod
    def setUpClass(cls) -> None:
        apply_migrations(os.environ["DATABASE_URL"])

    def setUp(self) -> None:
        self.token = uuid.uuid4().hex[:8]
        self.meet = f"__test_client_{self.token}__"
        self.client = IngestClient()

    def tearDown(self) -> None:
        with psycopg.connect(os.environ["DATABASE_URL"], autocommit=True) as conn:
            conn.execute("DELETE FROM session_schedule WHERE meet = %s", (self.meet,))

    def _count(self) -> int:
        with psycopg.connect(os.environ["DATABASE_URL"], row_factory=dict_row) as conn:
            return conn.execute(
                "SELECT COUNT(*) AS c FROM session_schedule WHERE meet = %s", (self.meet,)
            ).fetchone()["c"]

    def test_action_commits_one_row(self) -> None:
        result = self.client.action(
            "scraperIngestion:ingestSessionSchedule", _schedule(self.meet, 1, "Red")
        )
        self.assertTrue(result["wasInsert"])
        self.assertEqual(self._count(), 1)

    def test_actions_commits_the_batch_in_order(self) -> None:
        results = self.client.actions(
            "scraperIngestion:ingestSessionSchedule",
            [_schedule(self.meet, 1, "Red"), _schedule(self.meet, 2, "White")],
        )
        self.assertEqual([r["wasInsert"] for r in results], [True, True])
        self.assertEqual(self._count(), 2)
        self.assertEqual(self.client.actions("scraperIngestion:ingestSessionSchedule", []), [])

    def test_actions_rolls_back_the_whole_batch_on_a_failing_row(self) -> None:
        bad = _schedule(self.meet, 2, "White")
        bad["sessionId"] = "not-a-number"  # DOUBLE PRECISION column: Postgres raises
        with self.assertRaises(psycopg.Error):
            self.client.actions(
                "scraperIngestion:ingestSessionSchedule",
                [_schedule(self.meet, 1, "Red"), bad],
            )
        self.assertEqual(self._count(), 0)

    def test_actions_skipping_errors_keeps_the_good_rows(self) -> None:
        bad = _schedule(self.meet, 2, "White")
        bad["sessionId"] = "not-a-number"  # DOUBLE PRECISION column: Postgres raises
        with self.assertLogs(level="ERROR"):
            results = self.client.actions_skipping_errors(
                "scraperIngestion:ingestSessionSchedule",
                [_schedule(self.meet, 1, "Red"), bad, _schedule(self.meet, 3, "Blue")],
            )
        self.assertTrue(results[0]["wasInsert"])
        self.assertIsInstance(results[1], RowFailure)
        self.assertEqual(results[1].index, 1)
        self.assertTrue(results[2]["wasInsert"])
        # The failing row's savepoint was rolled back; the rows either side
        # of it were committed.
        self.assertEqual(self._count(), 2)

    def test_actions_rejects_an_unknown_path_before_writing(self) -> None:
        with self.assertRaises(NotImplementedError):
            self.client.actions("scraperIngestion:notARealAction", [{"meet": self.meet}])
