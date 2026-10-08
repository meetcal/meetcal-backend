#!/usr/bin/env python3

import os
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import utils  # noqa: E402


def record(weight_class, snatch=50, age="Senior", gender="Women"):
    return {
        "wso": "Ohio",
        "age_category": age,
        "gender": gender,
        "weight_class": weight_class,
        "snatch_record": snatch,
        "cj_record": None,
        "total_record": 0,
    }


class WsoPayloadTests(unittest.TestCase):
    def test_one_row_per_class_the_last_listed_winning(self):
        rows = utils.wso_payload(
            [record("77", snatch=120), record("81"), record("77", snatch=124)]
        )
        self.assertEqual(
            rows,
            [
                {"ageCategory": "Senior", "gender": "Women", "weightClass": "81", "snatchRecord": 50, "totalRecord": 0},
                {"ageCategory": "Senior", "gender": "Women", "weightClass": "77", "snatchRecord": 124, "totalRecord": 0},
            ],
        )

    def test_classes_compare_as_stored_after_normalizing(self):
        # "senior" and "F" are stored as "Senior" and "Women": one class.
        rows = utils.wso_payload(
            [record("77", snatch=120, age="senior", gender="F"), record("77", snatch=124)]
        )
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["snatchRecord"], 124)


class EveryPartTests(unittest.TestCase):
    def test_refuses_a_part_that_parsed_to_nothing(self):
        with self.assertRaisesRegex(ValueError, "Ohio: parsed 0 records from tab Masters Men"):
            utils.every_part("Ohio", [("tab Senior Women", [record("77")]), ("tab Masters Men", [])])

    def test_joins_the_parts(self):
        parts = [("a", [record("77")]), ("b", [record("81"), record("87")])]
        self.assertEqual(len(utils.every_part("Ohio", parts)), 3)


class SyncWsoRecordsTests(unittest.TestCase):
    def test_refuses_zero_records(self):
        with self.assertRaisesRegex(ValueError, "parsed 0 records"):
            utils.sync_wso_records("California North", [])

    def test_dry_run_writes_nothing(self):
        with patch("common.postgres_ingest.IngestClient") as client:
            self.assertIsNone(utils.sync_wso_records("Ohio", [record("77")], dry_run=True))
        client.assert_not_called()

    def test_one_exact_set_write(self):
        counts = {"inserted": 1, "updated": 0, "deleted": 2, "unchanged": 0}
        with patch("common.postgres_ingest.IngestClient") as client:
            client.return_value.action.return_value = counts
            result = utils.sync_wso_records(
                "Ohio", [record("77"), record("81")], allow_shrink=True
            )
        self.assertEqual(result, counts)
        client.return_value.action.assert_called_once()
        path, payload = client.return_value.action.call_args.args
        self.assertEqual(path, "scraperIngestion:replaceWSORecordSet")
        self.assertEqual(payload["wso"], "Ohio")
        self.assertTrue(payload["allowShrink"])
        self.assertEqual([row["weightClass"] for row in payload["records"]], ["77", "81"])


if __name__ == "__main__":
    unittest.main()
