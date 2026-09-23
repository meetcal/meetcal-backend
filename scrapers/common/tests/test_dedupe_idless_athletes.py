"""DB-backed tests for the one-off id-less athlete dedupe. Requires
DATABASE_URL; skips otherwise. Runs inside a rolled-back transaction."""

from __future__ import annotations

import os
import unittest
import uuid

try:
    import psycopg
    from psycopg.rows import dict_row

    from common import dedupe_idless_athletes as dedupe
except ImportError:  # pragma: no cover - optional local dep
    psycopg = None
    dict_row = None
    dedupe = None


@unittest.skipUnless(
    os.getenv("DATABASE_URL") and psycopg is not None,
    "DATABASE_URL and psycopg are required",
)
class DedupeIdlessAthletesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.token = uuid.uuid4().hex[:8]
        self.meet = f"__test_dedupe_{self.token}__"
        self.conn = psycopg.connect(os.environ["DATABASE_URL"], row_factory=dict_row)

    def tearDown(self) -> None:
        self.conn.rollback()
        self.conn.close()

    def _insert(self, name: str, member_id: str, meet: str | None = None, **extra) -> int:
        row = {
            "convex_id": f"athlete_{uuid.uuid4().hex}",
            "member_id": member_id,
            "name": name,
            "age": 25,
            "club": "Club",
            "gender": "Female",
            "weight_class": "71",
            "entry_total": 200,
            "session_number": None,
            "session_platform": None,
            "meet": meet or self.meet,
        }
        row.update(extra)
        return self.conn.execute(
            """
            INSERT INTO athletes (
                convex_id, member_id, name, age, club, gender, weight_class,
                entry_total, session_number, session_platform, meet
            )
            VALUES (
                %(convex_id)s, %(member_id)s, %(name)s, %(age)s, %(club)s, %(gender)s,
                %(weight_class)s, %(entry_total)s, %(session_number)s,
                %(session_platform)s, %(meet)s
            )
            RETURNING id
            """,
            row,
        ).fetchone()["id"]

    def _rows(self):
        return self.conn.execute(
            "SELECT id, member_id, name, session_number FROM athletes WHERE meet = %s ORDER BY id",
            (self.meet,),
        ).fetchall()

    def test_collapses_random_id_duplicates_and_keeps_the_assigned_row(self) -> None:
        older = self._insert("Jane Doe", "512345678")
        assigned = self._insert("jane doe", "612345678", session_number=4, session_platform="Red")
        newest = self._insert("Jane  Doe", "712345678")
        # A random-looking id that also appears at another meet is treated as
        # a real membership number: that group is ambiguous and left alone.
        seen_elsewhere = self._insert("Chris Roe", "912345678")
        self._insert("Chris Roe", "912345678", meet=f"{self.meet}_other")
        chris_dup = self._insert("Chris Roe", "922345678")
        # A group mixing a real id with random ones is ambiguous too.
        mixed_real = self._insert("Alex Kim", "1234567")
        mixed_random = self._insert("Alex Kim", "932345678")
        # A different athlete altogether.
        other = self._insert("Someone Else", "812345678")

        dry = dedupe.dedupe_meet(self.conn, self.meet, apply=False)
        self.assertEqual(dry["deleted"], 0)
        self.assertEqual(len(dry["groups"]), 1)
        self.assertEqual(dry["groups"][0]["keep_id"], assigned)
        self.assertEqual(sorted(dry["groups"][0]["delete_ids"]), [older, newest])
        self.assertEqual(len(self._rows()), 8)

        applied = dedupe.dedupe_meet(self.conn, self.meet, apply=True)
        self.assertEqual(applied["deleted"], 2)
        rows = {row["id"]: row for row in self._rows()}
        self.assertEqual(
            set(rows),
            {assigned, seen_elsewhere, chris_dup, mixed_real, mixed_random, other},
        )
        self.assertEqual(rows[assigned]["member_id"], "noid:jane-doe")
        self.assertEqual(float(rows[assigned]["session_number"]), 4.0)
        self.assertEqual(rows[seen_elsewhere]["member_id"], "912345678")
        self.assertEqual(rows[mixed_real]["member_id"], "1234567")

        # Idempotent: nothing left to collapse.
        again = dedupe.dedupe_meet(self.conn, self.meet, apply=True)
        self.assertEqual(again["groups"], [])
        self.assertEqual(again["deleted"], 0)

    def test_keeps_newest_when_no_row_has_a_session(self) -> None:
        self._insert("Pat Lee", "")
        newest = self._insert("Pat Lee", "noid:pat-lee")
        applied = dedupe.dedupe_meet(self.conn, self.meet, apply=True)
        self.assertEqual(applied["deleted"], 1)
        rows = self._rows()
        self.assertEqual([row["id"] for row in rows], [newest])
        self.assertEqual(rows[0]["member_id"], "noid:pat-lee")

    def test_refuses_an_empty_meet(self) -> None:
        with self.assertRaisesRegex(ValueError, "meet is required"):
            dedupe.dedupe_meet(self.conn, "  ", apply=True)


if __name__ == "__main__":
    unittest.main()
