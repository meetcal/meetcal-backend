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


if __name__ == "__main__":
    unittest.main()
