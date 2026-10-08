#!/usr/bin/env python3
"""Florida WSO records scraper: every tab is read or the run fails, then one exact-set sync."""

import contextlib
import io
import os
import sys
import unittest
from unittest import mock

SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import requests  # noqa: E402

import scraper_florida  # noqa: E402
from utils import UNKNOWN_GID  # noqa: E402

# What gviz answers an unknown gid with: the sheet's first tab.
FIRST_TAB_CSV = '"The sheet\'s first tab"\n'

SHEET_URL = "https://docs.google.com/spreadsheets/d/16sNrOTnGrGeXE4L5skgCfE5vLTA7ggpaHWfMQNh0DfQ/view?gid=490899077#gid=490899077"

# The Senior tab (gid 662417948) as gviz serves it, first two classes, trailing blank columns cut.
SENIOR_CSV = """\
"Senior State Records ","","","","","","","","","","",""
"60","Snatch","102","Michael Tucciarone","6/20/26","","48","Snatch","69","STANDARD","6/1/25",""
"","Clean and Jerk","128","Samuel Lewis","6/20/26","","","Clean & Jerk","88","STANDARD","6/1/25",""
"","Total","229","Samuel Lewis","6/20/26","","","Total","158","STANDARD","6/1/25",""
"65","Snatch","125","Bryson Brown","6/26/26","","53","Snatch","80","Asia Gonzalez","7/13/25",""
"","Clean and Jerk","147","Bryson Brown","4/10/26","","","Clean & Jerk","103","Asia Gonzalez","7/13/25",""
"","Total","265","Bryson Brown","6/26/26","","","Total","183","Asia Gonzalez","7/13/25",""
"""


class FakeResponse:
    def __init__(self, text: str, status_code: int = 200):
        self.text = text
        self.status_code = status_code

    def raise_for_status(self):
        if self.status_code >= 400:
            raise requests.HTTPError(f"{self.status_code} Server Error")


def serve(overrides=None):
    """A requests.get stand-in: every tab serves SENIOR_CSV unless its gid is overridden."""
    overrides = overrides or {}
    requested = []

    def get(url, timeout=None):
        gid = url.rsplit("gid=", 1)[1]
        if gid == UNKNOWN_GID:
            return overrides.get(gid, FakeResponse(FIRST_TAB_CSV))
        requested.append(gid)
        return overrides.get(gid, FakeResponse(SENIOR_CSV))

    get.requested = requested
    return get


def quietly(fn, *args, **kwargs):
    with contextlib.redirect_stdout(io.StringIO()):
        return fn(*args, **kwargs)


class FloridaScraperTests(unittest.TestCase):
    def setUp(self):
        self.scraper = scraper_florida.WSORecordsFloridaScraper("Florida", SHEET_URL)

    def scrape(self, get):
        with mock.patch.object(scraper_florida.requests, "get", side_effect=get):
            return quietly(self.scraper.scrape_sheet)

    def test_reads_every_tab(self):
        get = serve()
        records = self.scrape(get)

        self.assertEqual(get.requested, list(self.scraper.tabs.values()))
        self.assertEqual(len(records), 4 * len(self.scraper.tabs))
        self.assertEqual({r["age_category"] for r in records}, set(self.scraper.tabs))
        senior_men_60 = next(
            r for r in records
            if (r["age_category"], r["gender"], r["weight_class"]) == ("Senior", "Men", "60")
        )
        self.assertEqual(
            (senior_men_60["snatch_record"], senior_men_60["cj_record"], senior_men_60["total_record"]),
            (102, 128, 229),
        )

    def test_failing_tab_raises(self):
        with self.assertRaisesRegex(Exception, "500"):
            self.scrape(serve({"1300164988": FakeResponse("", status_code=500)}))

    def test_empty_tab_raises(self):
        with self.assertRaisesRegex(ValueError, "Masters 90"):
            self.scrape(serve({"575067900": FakeResponse('"Masters 90 State Records "\n')}))

    def test_deleted_tab_served_as_the_first_tab_raises(self):
        # Google answers a deleted tab's gid with the sheet's first tab, HTTP 200.
        with self.assertRaisesRegex(ValueError, "tab gid 1300164988 returned the sheet's first tab"):
            self.scrape(serve({"1300164988": FakeResponse(FIRST_TAB_CSV)}))

    def test_run_syncs_once_with_every_record(self):
        argv = ["scraper_florida.py", "--wso", "Florida", "--sheet-url", SHEET_URL, "--allow-shrink"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.dict(os.environ, {"SLACK_WEBHOOK_URL": ""}), \
                mock.patch.object(scraper_florida.requests, "get", side_effect=serve()), \
                mock.patch.object(scraper_florida, "sync_wso_records") as sync:
            quietly(scraper_florida.main)

        sync.assert_called_once()
        (wso, records), kwargs = sync.call_args
        self.assertEqual(wso, "Florida")
        self.assertEqual(len(records), 4 * len(self.scraper.tabs))
        self.assertEqual(kwargs, {"dry_run": False, "allow_shrink": True})

    def test_dry_run_touches_no_database(self):
        argv = ["scraper_florida.py", "--wso", "Florida", "--sheet-url", SHEET_URL, "--dry-run"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.object(scraper_florida.requests, "get", side_effect=serve()), \
                mock.patch("common.postgres_ingest.IngestClient") as ingest_client:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                scraper_florida.main()

        ingest_client.assert_not_called()
        self.assertIn("Dry run: would sync 68 Florida classes", output.getvalue())


if __name__ == "__main__":
    unittest.main()
