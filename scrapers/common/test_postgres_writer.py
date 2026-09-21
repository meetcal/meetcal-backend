#!/usr/bin/env python3

import unittest
from unittest.mock import patch

from common import postgres_writer


class QueryResult:
    def __init__(self, rows=None, rowcount=0):
        self.rows = rows or []
        self.rowcount = rowcount

    def fetchall(self):
        return self.rows


class FakeConnection:
    def __init__(self, existing_rows):
        self.existing_rows = existing_rows
        self.deleted_ids = []

    def execute(self, query, params=None):
        normalized = " ".join(query.split()).upper()
        if normalized.startswith("SELECT"):
            return QueryResult(self.existing_rows)
        if normalized.startswith("DELETE FROM WSO_RECORDS"):
            self.deleted_ids.append(params[0])
            return QueryResult()
        if normalized.startswith("DELETE FROM INTL_RANKINGS"):
            self.deleted_ids.append(params[0])
            return QueryResult()
        raise AssertionError(f"Unexpected query: {query}")


class PayloadCleaningTests(unittest.TestCase):
    """`scraperSecret` is a transport credential, never a column. Every
    `upsert_*` runs its row through `clean` before building values."""

    def test_clean_strips_the_scraper_secret_and_keeps_everything_else(self):
        cleaned = postgres_writer.clean(
            {"scraperSecret": "hunter2", "name": "Ada", "total": 200}
        )
        self.assertEqual(cleaned, {"name": "Ada", "total": 200})

    def test_every_upsert_cleans_its_row(self):
        import inspect

        upserts = [
            name
            for name in dir(postgres_writer)
            if name.startswith("upsert_") and callable(getattr(postgres_writer, name))
        ]
        self.assertTrue(upserts)
        for name in upserts:
            source = inspect.getsource(getattr(postgres_writer, name))
            self.assertIn("clean(row)", source, f"{name} does not clean its payload")


class ReplaceWsoRecordsTests(unittest.TestCase):
    def test_exact_set_sync_counts_and_writes_only_changes(self):
        existing_rows = [
            {
                "convex_id": "unchanged-id",
                "wso": "Illinois",
                "age_category": "U13",
                "gender": "Women",
                "weight_class": "30",
                "snatch_record": 0,
                "cj_record": 0,
                "total_record": 0,
            },
            {
                "convex_id": "updated-id",
                "wso": "Illinois",
                "age_category": "U13",
                "gender": "Women",
                "weight_class": "37",
                "snatch_record": 9,
                "cj_record": 13,
                "total_record": 24,
            },
            {
                "convex_id": "deleted-id",
                "wso": "Illinois",
                "age_category": "U11",
                "gender": "Women",
                "weight_class": "30",
                "snatch_record": 1,
                "cj_record": 1,
                "total_record": 2,
            },
        ]
        incoming_rows = [
            {
                "ageCategory": "U13",
                "gender": "Women",
                "weightClass": "30",
                "snatchRecord": 0,
                "cjRecord": 0,
                "totalRecord": 0,
            },
            {
                "ageCategory": "U13",
                "gender": "Women",
                "weightClass": "37",
                "snatchRecord": 10,
                "cjRecord": 13,
                "totalRecord": 24,
            },
            {
                "ageCategory": "U13",
                "gender": "Women",
                "weightClass": "41",
                "snatchRecord": 28,
                "cjRecord": 43,
                "totalRecord": 70,
            },
        ]
        connection = FakeConnection(existing_rows)

        with patch.object(postgres_writer, "upsert_wso_record") as upsert:
            result = postgres_writer.replace_wso_records(
                connection, "Illinois", incoming_rows
            )

        self.assertEqual(
            result,
            {"inserted": 1, "updated": 1, "unchanged": 1, "deleted": 1},
        )
        self.assertEqual(connection.deleted_ids, ["deleted-id"])
        self.assertEqual(upsert.call_count, 2)
        written_weights = {call.args[1]["weightClass"] for call in upsert.call_args_list}
        self.assertEqual(written_weights, {"37", "41"})

    def test_rejects_duplicate_incoming_keys(self):
        connection = FakeConnection([])
        duplicate = {
            "ageCategory": "U13",
            "gender": "Women",
            "weightClass": "30",
            "snatchRecord": 0,
        }

        with self.assertRaisesRegex(ValueError, "Duplicate WSO record in payload"):
            postgres_writer.replace_wso_records(
                connection, "Illinois", [duplicate, duplicate]
            )

        self.assertEqual(connection.deleted_ids, [])


class ReplaceWsoRecordsGuardTests(unittest.TestCase):
    def test_rejects_empty_wso(self):
        connection = FakeConnection([])
        with self.assertRaisesRegex(ValueError, "wso is required"):
            postgres_writer.replace_wso_records(connection, "", [])
        self.assertEqual(connection.deleted_ids, [])

    def test_rejects_empty_payload(self):
        # An exact-set sync with zero incoming rows deletes every existing row
        # for the WSO, so a failed PDF parse used to wipe the record set.
        connection = FakeConnection(
            [
                {
                    "convex_id": "existing-id",
                    "wso": "Illinois",
                    "age_category": "Senior",
                    "gender": "Women",
                    "weight_class": "71",
                    "snatch_record": 80,
                    "cj_record": 100,
                    "total_record": 180,
                }
            ]
        )
        with self.assertRaisesRegex(ValueError, "empty payload"):
            postgres_writer.replace_wso_records(connection, "Illinois", [])
        self.assertEqual(connection.deleted_ids, [])


class ReplaceIntlRankingsTests(unittest.TestCase):
    def test_exact_set_sync_counts_and_writes_only_changes(self):
        existing_rows = [
            {
                "convex_id": "unchanged-id",
                "legacy_id": None,
                "meet": "Worlds",
                "ranking": 1,
                "name": "Keep",
                "weight_class": "71",
                "total": 200,
                "percent_a": 90,
                "gender": "Women",
                "age_category": "Senior",
            },
            {
                "convex_id": "updated-id",
                "legacy_id": None,
                "meet": "Worlds",
                "ranking": 2,
                "name": "Update",
                "weight_class": "71",
                "total": 190,
                "percent_a": 80,
                "gender": "Women",
                "age_category": "Senior",
            },
            {
                "convex_id": "deleted-id",
                "legacy_id": None,
                "meet": "Worlds",
                "ranking": 3,
                "name": "Drop",
                "weight_class": "71",
                "total": 180,
                "percent_a": 70,
                "gender": "Women",
                "age_category": "Senior",
            },
        ]
        incoming_rows = [
            {
                "ranking": 1,
                "name": "Keep",
                "weightClass": "71",
                "total": 200,
                "percentA": 90,
            },
            {
                "ranking": 2,
                "name": "Update",
                "weightClass": "71",
                "total": 191,
                "percentA": 80,
            },
            {
                "ranking": 4,
                "name": "New",
                "weightClass": "71",
                "total": 170,
                "percentA": 60,
            },
        ]
        connection = FakeConnection(existing_rows)

        with patch.object(postgres_writer, "upsert_intl_ranking") as upsert:
            result = postgres_writer.replace_intl_rankings_group(
                connection,
                {
                    "meet": "Worlds",
                    "gender": "Women",
                    "ageCategory": "Senior",
                    "rankings": incoming_rows,
                },
            )

        self.assertEqual(
            result,
            {"inserted": 1, "updated": 1, "unchanged": 1, "deleted": 1},
        )
        self.assertEqual(connection.deleted_ids, ["deleted-id"])
        self.assertEqual(upsert.call_count, 2)

    def test_rejects_empty_group_identity(self):
        connection = FakeConnection([])
        with self.assertRaisesRegex(ValueError, "meet is required"):
            postgres_writer.replace_intl_rankings_group(
                connection, {"meet": "", "gender": "Women", "ageCategory": "Senior", "rankings": []}
            )

    def test_rejects_empty_rankings_payload(self):
        # Removing a group that genuinely disappeared is
        # `deleteMissingIntlRankingGroups`' job; an empty replace is a failed
        # scrape and must not delete the group.
        connection = FakeConnection(
            [
                {
                    "convex_id": "existing-id",
                    "legacy_id": None,
                    "meet": "Worlds",
                    "ranking": 1,
                    "name": "Keep",
                    "weight_class": "71",
                    "total": 200,
                    "percent_a": 90,
                    "gender": "Women",
                    "age_category": "Senior",
                }
            ]
        )
        with self.assertRaisesRegex(ValueError, "empty payload"):
            postgres_writer.replace_intl_rankings_group(
                connection,
                {"meet": "Worlds", "gender": "Women", "ageCategory": "Senior", "rankings": []},
            )
        self.assertEqual(connection.deleted_ids, [])

    def test_rejects_duplicate_incoming_keys(self):
        connection = FakeConnection([])
        duplicate = {"ranking": 1, "name": "Ada", "weightClass": "71"}
        with self.assertRaisesRegex(ValueError, "Duplicate intl ranking in payload"):
            postgres_writer.replace_intl_rankings_group(
                connection,
                {
                    "meet": "Worlds",
                    "gender": "Women",
                    "ageCategory": "Senior",
                    "rankings": [duplicate, duplicate],
                },
            )
        self.assertEqual(connection.deleted_ids, [])


class DestructiveWriteGuardTests(unittest.TestCase):
    """Delete-by-meet and wholesale replace live in the writer and must refuse
    an empty key / empty payload, whoever calls them."""

    class _RecordingConnection:
        def __init__(self):
            self.statements = []

        def execute(self, query, params=None):
            self.statements.append((" ".join(query.split()), params))
            return QueryResult(rowcount=1)

    def test_delete_athletes_by_meet_requires_meet(self):
        connection = self._RecordingConnection()
        for empty in ("", "   ", None, 5):
            with self.assertRaisesRegex(ValueError, "meet is required"):
                postgres_writer.delete_athletes_by_meet(connection, empty)
        self.assertEqual(connection.statements, [])

    def test_delete_session_schedule_by_meet_requires_meet(self):
        connection = self._RecordingConnection()
        for empty in ("", "   ", None):
            with self.assertRaisesRegex(ValueError, "meet is required"):
                postgres_writer.delete_session_schedule_by_meet(connection, empty)
        self.assertEqual(connection.statements, [])

    def test_delete_by_meet_scopes_the_statement_to_the_meet(self):
        connection = self._RecordingConnection()
        self.assertEqual(
            postgres_writer.delete_athletes_by_meet(connection, "2026 Nationals"), 1
        )
        self.assertEqual(
            postgres_writer.delete_session_schedule_by_meet(connection, "2026 Nationals"), 1
        )
        self.assertEqual(
            [statement for statement, _ in connection.statements],
            [
                "DELETE FROM athletes WHERE meet = %s RETURNING 1",
                "DELETE FROM session_schedule WHERE meet = %s RETURNING 1",
            ],
        )
        self.assertEqual(
            [params for _, params in connection.statements],
            [("2026 Nationals",), ("2026 Nationals",)],
        )

    def test_replace_records_refuses_empty_payload(self):
        # `replaceIWFRecords` with `records: []` used to DELETE every IWF record
        # and insert nothing, wiping the set on a failed scrape.
        connection = self._RecordingConnection()
        with self.assertRaisesRegex(ValueError, "empty payload"):
            postgres_writer.replace_records(connection, "IWF", [])
        self.assertEqual(connection.statements, [])

    def test_replace_records_requires_a_record_type(self):
        connection = self._RecordingConnection()
        for empty in ("", "   ", None):
            with self.assertRaisesRegex(ValueError, "recordType is required"):
                postgres_writer.replace_records(connection, empty, [{"weightClass": "61kg"}])
        self.assertEqual(connection.statements, [])

    def test_replace_records_scopes_the_delete_to_the_record_type(self):
        connection = self._RecordingConnection()
        with patch.object(postgres_writer, "upsert_record") as upsert:
            result = postgres_writer.replace_records(
                connection, "IWF", [{"weightClass": "61kg"}, {"weightClass": "73kg"}]
            )

        self.assertEqual(result, {"deleted": True, "inserted": 2})
        self.assertEqual(
            connection.statements,
            [("DELETE FROM records WHERE record_type = %s", ("IWF",))],
        )
        self.assertEqual(
            [call.args[1]["recordType"] for call in upsert.call_args_list],
            ["IWF", "IWF"],
        )

    def test_replace_all_intl_rankings_refuses_empty_payload(self):
        connection = self._RecordingConnection()
        with self.assertRaisesRegex(ValueError, "empty payload"):
            postgres_writer.replace_all_intl_rankings(connection, [])
        self.assertEqual(connection.statements, [])


if __name__ == "__main__":
    unittest.main()
