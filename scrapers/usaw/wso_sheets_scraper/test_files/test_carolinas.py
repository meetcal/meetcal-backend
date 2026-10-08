#!/usr/bin/env python3
"""Carolina WSO records scraper: every tab is read or the run fails, then one exact-set sync."""

import contextlib
import io
import os
import sys
import unittest
from unittest import mock

SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import requests  # noqa: E402

import scraper_carolinas  # noqa: E402
from utils import UNKNOWN_GID  # noqa: E402

# What gviz answers an unknown gid with: the sheet's first tab.
FIRST_TAB_CSV = '"The sheet\'s first tab"\n'

SHEET_URL = "https://docs.google.com/spreadsheets/d/1rKFzpkLCT-FE2SzM0qpUOoZ788YHl7dg/view?gid=1785893123#gid=1785893123"

# The SENIOR tab (gid 2109027801) as gviz serves it, first two classes, trailing blank columns cut.
SENIOR_CSV = """\
"Senior WSO Records Men's Records Cateogry","Lift","Weight","Athlete","Club","Date","Location","","Women's Records Category","Lift","Weight","Athlete","Club","Date","Location"
"60","Snatch","94","WSO Standard","","","","","48","Snatch","65","WSO Standard","","",""
"","C & J","131","WSO Standard","","","","","","C & J","83","WSO Standard","","",""
"","Total","226","WSO Standard","","","","","","Total","149","WSO Standard","","",""
"65","Snatch","107","Chase Overpeck","Athletic Lab WLC","6/26/26","Colorado Springs, CO ","","53","Snatch","77","Katherine Lee","Harrisburg Weightlifting Club","6/27/25","Colorado Springs, CO"
"","C & J","132","WSO Standard","","","","","","C & J","92","Sarah Wright","Greensboro Barbell","3/5/26","Columbus, OH"
"","Total","232","Chase Overpeck","Athletic Lab WLC","6/26/26","Colorado Springs, CO ","","","Total","168","Sarah Wright","Greensboro Barbell","12/5/25","Daytona Beach, FL"
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


class CarolinasScraperTests(unittest.TestCase):
    def setUp(self):
        self.scraper = scraper_carolinas.WSORecordsCarolinasScraper("Carolina", SHEET_URL)

    def scrape(self, get):
        with mock.patch.object(scraper_carolinas.requests, "get", side_effect=get):
            return quietly(self.scraper.scrape_sheet)

    def test_reads_every_tab(self):
        get = serve()
        records = self.scrape(get)

        self.assertEqual(get.requested, list(self.scraper.tabs.values()))
        self.assertEqual(len(records), 4 * len(self.scraper.tabs))
        # The Youth and Masters tabs stack age groups; one section reads as the first of each.
        self.assertEqual({r["age_category"] for r in records}, {"U13", "Junior", "Senior", "Masters 35"})
        senior_women_53 = next(
            r for r in records
            if (r["age_category"], r["gender"], r["weight_class"]) == ("Senior", "Women", "53")
        )
        self.assertEqual(
            (senior_women_53["snatch_record"], senior_women_53["cj_record"], senior_women_53["total_record"]),
            (77, 92, 168),
        )

    def test_failing_tab_raises(self):
        with self.assertRaisesRegex(Exception, "500"):
            self.scrape(serve({"1157313505": FakeResponse("", status_code=500)}))

    def test_empty_tab_raises(self):
        with self.assertRaisesRegex(ValueError, "MASTER"):
            self.scrape(serve({"448005775": FakeResponse('"Masters WSO Records"\n')}))

    def test_deleted_tab_served_as_the_first_tab_raises(self):
        # Google answers a deleted tab's gid with the sheet's first tab, HTTP 200.
        with self.assertRaisesRegex(ValueError, "tab gid 1157313505 returned the sheet's first tab"):
            self.scrape(serve({"1157313505": FakeResponse(FIRST_TAB_CSV)}))

    def test_youth_tab_may_be_the_first_tab(self):
        # The Youth tab is the sheet's first, so it matches the unknown-gid answer.
        get = serve({UNKNOWN_GID: FakeResponse(SENIOR_CSV)})
        with self.assertRaisesRegex(ValueError, "tab gid 1157313505 returned the sheet's first tab"):
            self.scrape(get)
        youth_only = serve({UNKNOWN_GID: FakeResponse(SENIOR_CSV.replace("Senior", "Youth")), "1785893123": FakeResponse(SENIOR_CSV.replace("Senior", "Youth"))})
        self.assertEqual(len(self.scrape(youth_only)), 4 * len(self.scraper.tabs))

    def test_run_syncs_once_with_every_record(self):
        argv = ["scraper_carolinas.py", "--wso", "Carolina", "--sheet-url", SHEET_URL, "--allow-shrink"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.dict(os.environ, {"SLACK_WEBHOOK_URL": ""}), \
                mock.patch.object(scraper_carolinas.requests, "get", side_effect=serve()), \
                mock.patch.object(scraper_carolinas, "sync_wso_records") as sync:
            quietly(scraper_carolinas.main)

        sync.assert_called_once()
        (wso, records), kwargs = sync.call_args
        self.assertEqual(wso, "Carolina")
        self.assertEqual(len(records), 4 * len(self.scraper.tabs))
        self.assertEqual(kwargs, {"dry_run": False, "allow_shrink": True})

    def test_dry_run_touches_no_database(self):
        argv = ["scraper_carolinas.py", "--wso", "Carolina", "--sheet-url", SHEET_URL, "--dry-run"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.object(scraper_carolinas.requests, "get", side_effect=serve()), \
                mock.patch("common.postgres_ingest.IngestClient") as ingest_client:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                scraper_carolinas.main()

        ingest_client.assert_not_called()
        self.assertIn("Dry run: would sync 16 Carolina classes", output.getvalue())


if __name__ == "__main__":
    unittest.main()
