#!/usr/bin/env python3
"""Tennessee-Kentucky WSO records scraper: an unreadable sheet fails the run, then one exact-set sync."""

import contextlib
import io
import os
import sys
import unittest
from unittest import mock

SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import scraper_tnky  # noqa: E402

SHEET_URL = "https://docs.google.com/spreadsheets/d/11uUA0t05sEvHRjvDksC0VP1Yr2p_rC0JjHgVPEuYzhU/view?gid=867133960#gid=867133960"

# The sheet's first section (Youth Men 13 & Under) as gviz serves it, with
# the next section's header after it; the header's long note is cut short.
SHEET_CSV = """\
"TN-KY WSO RECORDS YOUTH: MEN ","40 KG","13 & Under 44 KG","48 KG","52 KG","56 KG","60 KG","65KG","65+ KG","","","Updated: 10/18/25"
"SNATCH","27","27","30","","","47","","56","","",""
"Name","Wyatt Cundiff","Max Wheeler","William Wheeler","","","HAYHOE, Gideon","","Noah Wilhoit","","",""
"Date","10/18/2025","10/18/2025","10/18/2025","","","12/6/2025","","10/18/2025","","",""
"C&J","30","30","35","","","60","","65","","",""
"Name","Wyatt Cundiff","Max Wheeler","William Wheeler","","","HAYHOE, Gideon","","Noah Wilhoit","","",""
"Date","10/18/2025","10/18/2025","10/18/2025","","","12/6/2025","","10/18/2025","","",""
"TOTAL","57","57","65","","","107","","121","","",""
"Name","Wyatt Cundiff","Max Wheeler","William Wheeler","","","HAYHOE, Gideon","","Noah Wilhoit","","",""
"Date","10/18/2025","10/18/2025","10/18/2025","","","12/6/2025","","10/18/2025","","",""
"YOUTH: WOMEN ","","13 & Under","","","","","","","","",""
"""


class FakeResponse:
    def __init__(self, text: str, status_code: int = 200):
        self.text = text
        self.status_code = status_code


def serve(response=None):
    """A requests.get stand-in serving SHEET_CSV, or ``response``."""
    requested = []

    def get(url, timeout=None):
        requested.append(url)
        return response or FakeResponse(SHEET_CSV)

    get.requested = requested
    return get


def quietly(fn, *args, **kwargs):
    with contextlib.redirect_stdout(io.StringIO()):
        return fn(*args, **kwargs)


class TnkyScraperTests(unittest.TestCase):
    def setUp(self):
        self.scraper = scraper_tnky.WSORecordsTNKYScraper("Tennessee-Kentucky", SHEET_URL)

    def scrape(self, get):
        with mock.patch.object(scraper_tnky.requests, "get", side_effect=get):
            return quietly(self.scraper.scrape_sheet)

    def test_reads_the_sheet(self):
        get = serve()
        records = self.scrape(get)

        self.assertEqual(len(get.requested), 1)
        self.assertTrue(get.requested[0].endswith("gid=867133960"))
        self.assertEqual(
            [r["weight_class"] for r in records], ["40", "44", "48", "52", "56", "60", "65", "65+"]
        )
        self.assertEqual({(r["age_category"], r["gender"]) for r in records}, {("U13", "Men")})
        by_class = {r["weight_class"]: (r["snatch_record"], r["cj_record"], r["total_record"]) for r in records}
        self.assertEqual(by_class["40"], (27, 30, 57))
        self.assertEqual(by_class["52"], (None, None, None))

    def test_failing_fetch_raises(self):
        with self.assertRaisesRegex(Exception, "500"):
            self.scrape(serve(FakeResponse("", status_code=500)))

    def test_empty_sheet_raises(self):
        with self.assertRaisesRegex(ValueError, "867133960"):
            self.scrape(serve(FakeResponse('"TN-KY WSO RECORDS"\n')))

    def test_run_syncs_once_with_every_record(self):
        argv = ["scraper_tnky.py", "--wso", "Tennessee-Kentucky", "--sheet-url", SHEET_URL, "--allow-shrink"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.dict(os.environ, {"SLACK_WEBHOOK_URL": ""}), \
                mock.patch.object(scraper_tnky.requests, "get", side_effect=serve()), \
                mock.patch.object(scraper_tnky, "sync_wso_records") as sync:
            quietly(scraper_tnky.main)

        sync.assert_called_once()
        (wso, records), kwargs = sync.call_args
        self.assertEqual(wso, "Tennessee-Kentucky")
        self.assertEqual(len(records), 8)
        self.assertEqual(kwargs, {"dry_run": False, "allow_shrink": True})

    def test_dry_run_touches_no_database(self):
        argv = ["scraper_tnky.py", "--wso", "Tennessee-Kentucky", "--sheet-url", SHEET_URL, "--dry-run"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.object(scraper_tnky.requests, "get", side_effect=serve()), \
                mock.patch("common.postgres_ingest.IngestClient") as ingest_client:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                scraper_tnky.main()

        ingest_client.assert_not_called()
        self.assertIn("Dry run: would sync 8 Tennessee-Kentucky classes", output.getvalue())


if __name__ == "__main__":
    unittest.main()
