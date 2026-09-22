#!/usr/bin/env python3

import unittest
from unittest.mock import patch

from common import postgres_writer


class QueryResult:
    def __init__(self, rows=None):
        self.rows = rows or []

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


class FetchResult:
    def __init__(self, row):
        self.row = row

    def fetchone(self):
        return self.row


class RecordingConnection:
    def __init__(self, existing):
        self.existing = existing
        self.statements = []

    def execute(self, query, params=None):
        statement = " ".join(query.split())
        self.statements.append(statement)
        upper = statement.upper()
        if upper.startswith("SELECT"):
            return FetchResult(self.existing)
        if upper.startswith("INSERT"):
            return FetchResult({"id": 42})
        raise AssertionError(statement)


def _entry_payload(**overrides):
    row = {
        "memberId": "55",
        "name": "Ada Lifter",
        "age": 22,
        "club": "New Club",
        "gender": "Female",
        "weightClass": "64",
        "entryTotal": 190,
        "meet": "Florida WSO 2026",
    }
    row.update(overrides)
    return row


def _stored_athlete(**overrides):
    row = {
        "id": 7,
        "convex_id": "athlete_existing",
        "member_id": "55",
        "name": "Ada Lifter",
        "age": 22,
        "club": "Old Club",
        "wso": None,
        "gender": "Female",
        "weight_class": "64",
        "entry_total": 180,
        "session_number": 3.0,
        "session_platform": "Red",
        "meet": "Florida WSO 2026",
        "adaptive": False,
    }
    row.update(overrides)
    return row


class AthleteSessionGateTests(unittest.TestCase):
    def test_session_assignment_is_non_null_number_or_platform(self):
        self.assertFalse(postgres_writer.athlete_has_session_assignment(None))
        self.assertFalse(
            postgres_writer.athlete_has_session_assignment(
                {"session_number": None, "session_platform": None}
            )
        )
        self.assertFalse(
            postgres_writer.athlete_has_session_assignment(
                {"session_number": None, "session_platform": "  "}
            )
        )
        self.assertTrue(
            postgres_writer.athlete_has_session_assignment(
                {"session_number": 0, "session_platform": None}
            )
        )
        self.assertTrue(
            postgres_writer.athlete_has_session_assignment(
                {"session_number": None, "session_platform": "Blue"}
            )
        )

    def test_entry_upsert_skips_row_when_session_already_set(self):
        connection = RecordingConnection(_stored_athlete())
        with self.assertLogs("common.postgres_writer", level="WARNING") as logs:
            result = postgres_writer.upsert_athlete(
                connection, _entry_payload(), preserve_assigned_session=True
            )

        self.assertEqual(
            result,
            {
                "id": "7",
                "wasInsert": False,
                "wasChanged": False,
                "skipped": True,
                "skipReason": "session already set",
            },
        )
        self.assertEqual(len(connection.statements), 1)
        self.assertIn("FOR UPDATE", connection.statements[0].upper())
        logged = "\n".join(logs.output)
        self.assertIn("session already set", logged)
        self.assertIn("meet=Florida WSO 2026", logged)
        self.assertIn("id=7", logged)
        self.assertIn("convex_id=athlete_existing", logged)
        self.assertNotIn("Ada Lifter", logged)
        self.assertNotIn("member_id", logged)

    def test_entry_upsert_skips_platform_only_assignment(self):
        connection = RecordingConnection(
            _stored_athlete(session_number=None, session_platform="Blue")
        )
        with self.assertLogs("common.postgres_writer", level="WARNING") as logs:
            result = postgres_writer.upsert_athlete(
                connection, _entry_payload(), preserve_assigned_session=True
            )
        self.assertTrue(result["skipped"])
        self.assertEqual(len(connection.statements), 1)
        self.assertTrue(any("session already set" in line for line in logs.output))

    def test_entry_upsert_still_inserts_and_updates_unassigned_athletes(self):
        missing = RecordingConnection(None)
        inserted = postgres_writer.upsert_athlete(
            missing, _entry_payload(), preserve_assigned_session=True
        )
        self.assertTrue(inserted["wasInsert"])
        self.assertFalse(inserted.get("skipped", False))
        insert_sql = next(stmt for stmt in missing.statements if stmt.upper().startswith("INSERT"))
        self.assertIn("THEN athletes.session_number", insert_sql)
        self.assertIn("THEN athletes.session_platform", insert_sql)

        open_row = RecordingConnection(
            _stored_athlete(session_number=None, session_platform=None)
        )
        updated = postgres_writer.upsert_athlete(
            open_row, _entry_payload(), preserve_assigned_session=True
        )
        self.assertFalse(updated["wasInsert"])
        self.assertTrue(updated["wasChanged"])
        self.assertFalse(updated.get("skipped", False))
        self.assertTrue(any(stmt.upper().startswith("INSERT") for stmt in open_row.statements))

    def test_start_list_upsert_still_overwrites_assigned_sessions(self):
        connection = RecordingConnection(_stored_athlete())
        result = postgres_writer.upsert_athlete(connection, _entry_payload())
        self.assertFalse(result.get("skipped", False))
        self.assertTrue(result["wasChanged"])
        self.assertNotIn("FOR UPDATE", connection.statements[0].upper())
        insert_sql = next(stmt for stmt in connection.statements if stmt.upper().startswith("INSERT"))
        self.assertIn("session_number = EXCLUDED.session_number", insert_sql)
        self.assertNotIn("THEN athletes.session_number", insert_sql)

    def test_ingest_paths_keep_the_gate_on_the_entry_scraper(self):
        from common.postgres_ingest import dispatch

        payload = _entry_payload()
        with patch.object(
            postgres_writer,
            "upsert_athlete",
            return_value={"id": "1", "wasInsert": False, "wasChanged": False},
        ) as upsert:
            dispatch(RecordingConnection(None), "scraperIngestion:ingestEntryAthlete", payload)
            dispatch(RecordingConnection(None), "scraperIngestion:ingestAthlete", payload)

        entry_call, start_list_call = upsert.call_args_list
        self.assertTrue(entry_call.kwargs["preserve_assigned_session"])
        self.assertFalse(start_list_call.kwargs.get("preserve_assigned_session", False))


if __name__ == "__main__":
    unittest.main()
