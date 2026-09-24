"""The `complete-ended-meets` cron SQL compares end_date against the meet's
local date, not the server's UTC date. Requires DATABASE_URL; skips otherwise.

Every check runs inside one transaction that is rolled back, so the seed
meets are never mutated.
"""

from __future__ import annotations

import os
import unittest
import uuid
from datetime import timedelta
from pathlib import Path

try:
    import psycopg
    from psycopg.rows import dict_row

    from common.tests.db_schema import apply_migrations
except ImportError:  # pragma: no cover - optional local dep
    psycopg = None
    dict_row = None

SQL_PATH = Path(__file__).resolve().parents[1] / "sql" / "complete_ended_meets.sql"
# Extreme offsets: the local date in UTC-12 and UTC+14 always differ, so the
# assertions hold at any time of day the test happens to run.
FAR_WEST = "Etc/GMT+12"  # POSIX sign: this is UTC-12
FAR_EAST = "Pacific/Kiritimati"  # UTC+14


@unittest.skipUnless(
    os.getenv("DATABASE_URL") and psycopg is not None,
    "DATABASE_URL and psycopg are required",
)
class CompleteEndedMeetsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        apply_migrations(os.environ["DATABASE_URL"])

    def setUp(self) -> None:
        self.sql = SQL_PATH.read_text(encoding="utf-8")
        self.token = uuid.uuid4().hex[:8]
        self.conn = psycopg.connect(os.environ["DATABASE_URL"], row_factory=dict_row)

    def tearDown(self) -> None:
        self.conn.rollback()
        self.conn.close()

    def _local_today(self, zone: str):
        return self.conn.execute(
            "SELECT (NOW() AT TIME ZONE %s)::date AS d", (zone,)
        ).fetchone()["d"]

    def _insert(self, label: str, zone: str, end_date) -> str:
        name = f"__test_ended_{label}_{self.token}__"
        self.conn.execute(
            """
            INSERT INTO meets (
                convex_id, name, federation, start_date, end_date, status, time_zone,
                updated_at, venue_name, venue_street, venue_city, venue_state, venue_zip
            )
            VALUES (%s, %s, 'USAW', %s, %s, 'upcoming', %s, 0, '', '', '', '', '')
            """,
            (f"meet_{label}_{self.token}", name, end_date, end_date, zone),
        )
        return name

    def _status(self, name: str) -> str:
        return self.conn.execute(
            "SELECT status FROM meets WHERE name = %s", (name,)
        ).fetchone()["status"]

    def test_uses_meet_local_date_and_tolerates_bad_zones(self) -> None:
        west_today = self._local_today(FAR_WEST)
        east_today = self._local_today(FAR_EAST)
        self.assertLess(west_today, east_today)

        # Still today in the far-west zone: must stay open even though the
        # UTC date (and every zone east of it) may already have rolled over.
        still_running = self._insert("west_today", FAR_WEST, west_today)
        # Ended yesterday in the far-east zone: complete it even when UTC has
        # not reached that date yet.
        ended_east = self._insert("east_yesterday", FAR_EAST, east_today - timedelta(days=1))
        # Unknown zone name falls back to UTC instead of erroring the statement.
        utc_today = self._local_today("UTC")
        bad_zone_open = self._insert("bad_open", "Mars/Olympus_Mons", utc_today)
        bad_zone_ended = self._insert("bad_ended", "Mars/Olympus_Mons", utc_today - timedelta(days=1))
        blank_zone_ended = self._insert("blank_ended", "", utc_today - timedelta(days=1))

        count = self.conn.execute(self.sql).fetchone()["count"]

        self.assertEqual(self._status(still_running), "upcoming")
        self.assertEqual(self._status(ended_east), "completed")
        self.assertEqual(self._status(bad_zone_open), "upcoming")
        self.assertEqual(self._status(bad_zone_ended), "completed")
        self.assertEqual(self._status(blank_zone_ended), "completed")
        self.assertGreaterEqual(count, 3)

    def test_is_idempotent(self) -> None:
        east_today = self._local_today(FAR_EAST)
        name = self._insert("twice", FAR_EAST, east_today - timedelta(days=2))
        self.conn.execute(self.sql).fetchone()
        first_updated_at = self.conn.execute(
            "SELECT updated_at FROM meets WHERE name = %s", (name,)
        ).fetchone()["updated_at"]
        self.conn.execute(self.sql).fetchone()
        row = self.conn.execute(
            "SELECT status, updated_at FROM meets WHERE name = %s", (name,)
        ).fetchone()
        self.assertEqual(row["status"], "completed")
        # A completed meet is not touched again.
        self.assertEqual(row["updated_at"], first_updated_at)


if __name__ == "__main__":
    unittest.main()
