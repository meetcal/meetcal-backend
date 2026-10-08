#!/usr/bin/env python3
"""New Jersey WSO records scraper: every tab is read or the run fails, then one exact-set sync."""

import contextlib
import io
import os
import sys
import unittest
from unittest import mock

SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import scraper_newjersey  # noqa: E402

SHEET_URL = "https://docs.google.com/spreadsheets/d/1y8mXDBLfqmszlzWhv-4wkeWQZS5Kb9Aj4RnB39CBJmw/edit?gid=0#gid=0"

# The Senior tab (gid 0) as gviz serves it, first four rows. Men's 71 is split
# over two rows (snatch on one, C&J and total on the other).
SENIOR_CSV = """\
"The standard rules of WSO/State records are as follows:","Weight Class","Athlete","Date/ Location","Snatch","Clean and Jerk","Total","","Weight Class","Athlete","Date/ Location","Snatch","Clean and Jerk","Total"
"i. The individual must reside in the WSO and be a member of the WSO.","48","Julie Carmody","Tri-State In House Meet July 20, 2007","36","49","85","","60","Maxson Salveo","10/25/2025 NJ WSO Championships","60","80","140"
"","53","Jennedyk Cabrera","10/25/2025 NJ WSO Championships","70","92","162","","65","Joe Delago","Moorestown Summer Classic, July 19, 2025","42","50","92"
"ii. Records maybe set at any USAW National meet (including Masters), International meet (including Masters (2023 or before) or meet held in the WSO. ","58","Tanya LaBell","10/25/2025 NJ WSO Championships","76","93","169","","71","Justin Bongcaron","USAW National U23 Championships, June 24, 2025","90","",""
"","63","Tanya LaBell","Tri-State In House Meet July 20, 2007","70","90","160","","71","Joshua Williams","10/25/2025 NJ WSO Championships","","124","214"
"""

CLASSES_PER_TAB = 7  # 4 women's, 3 men's once the split 71 is merged


class FakeResponse:
    def __init__(self, text: str, status_code: int = 200):
        self.text = text
        self.status_code = status_code


def serve(overrides=None):
    """A requests.get stand-in: every tab serves SENIOR_CSV unless its gid is overridden."""
    overrides = overrides or {}
    requested = []

    def get(url, timeout=None):
        gid = url.rsplit("gid=", 1)[1]
        requested.append(gid)
        return overrides.get(gid, FakeResponse(SENIOR_CSV))

    get.requested = requested
    return get


def quietly(fn, *args, **kwargs):
    with contextlib.redirect_stdout(io.StringIO()):
        return fn(*args, **kwargs)


class NewJerseyScraperTests(unittest.TestCase):
    def setUp(self):
        self.scraper = scraper_newjersey.WSORecordsNewJerseyScraper("New Jersey", SHEET_URL)

    def scrape(self, get):
        with mock.patch.object(scraper_newjersey.requests, "get", side_effect=get):
            return quietly(self.scraper.scrape_sheet)

    def test_reads_every_tab_and_merges_split_classes(self):
        get = serve()
        records = self.scrape(get)

        self.assertEqual(get.requested, list(self.scraper.tabs.values()))
        # gid 389932308 was the men's 80+ tab; it is the welcome page now.
        self.assertNotIn("389932308", get.requested)
        self.assertEqual(len(records), CLASSES_PER_TAB * len(self.scraper.tabs))
        senior_men_71 = [
            r for r in records
            if (r["age_category"], r["gender"], r["weight_class"]) == ("Senior", "Men", "71")
        ]
        self.assertEqual(len(senior_men_71), 1)
        self.assertEqual(
            (senior_men_71[0]["snatch_record"], senior_men_71[0]["cj_record"], senior_men_71[0]["total_record"]),
            (90, 124, 214),
        )

    def test_consolidate_keeps_each_lifts_best(self):
        row = {"wso": "New Jersey", "age_category": "Senior", "gender": "Men", "weight_class": "88"}
        merged = self.scraper._consolidate_records([
            {**row, "snatch_record": 125, "cj_record": None, "total_record": 281},
            {**row, "snatch_record": None, "cj_record": 157, "total_record": 270},
        ])
        self.assertEqual(merged, [{**row, "snatch_record": 125, "cj_record": 157, "total_record": 281}])

    def test_failing_tab_raises(self):
        with self.assertRaisesRegex(Exception, "500"):
            self.scrape(serve({"2116279815": FakeResponse("", status_code=500)}))

    def test_empty_tab_raises(self):
        # What gviz serves for a gid that became the welcome page: no row is wide enough.
        welcome = '"Senior Records (all ages)","","","The records begin on July 9, 2018.","","",""\n'
        with self.assertRaisesRegex(ValueError, "Masters 75"):
            self.scrape(serve({"2047529058": FakeResponse(welcome)}))

    def test_run_syncs_once_with_every_record(self):
        argv = ["scraper_newjersey.py", "--wso", "New Jersey", "--sheet-url", SHEET_URL, "--allow-shrink"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.dict(os.environ, {"SLACK_WEBHOOK_URL": ""}), \
                mock.patch.object(scraper_newjersey.requests, "get", side_effect=serve()), \
                mock.patch.object(scraper_newjersey, "sync_wso_records") as sync:
            quietly(scraper_newjersey.main)

        sync.assert_called_once()
        (wso, records), kwargs = sync.call_args
        self.assertEqual(wso, "New Jersey")
        self.assertEqual(len(records), CLASSES_PER_TAB * len(self.scraper.tabs))
        self.assertEqual(kwargs, {"dry_run": False, "allow_shrink": True})

    def test_dry_run_touches_no_database(self):
        argv = ["scraper_newjersey.py", "--wso", "New Jersey", "--sheet-url", SHEET_URL, "--dry-run"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.object(scraper_newjersey.requests, "get", side_effect=serve()), \
                mock.patch("common.postgres_ingest.IngestClient") as ingest_client:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                scraper_newjersey.main()

        ingest_client.assert_not_called()
        self.assertIn(
            f"Dry run: would sync {CLASSES_PER_TAB * len(self.scraper.tabs)} New Jersey classes",
            output.getvalue(),
        )


if __name__ == "__main__":
    unittest.main()
