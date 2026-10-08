#!/usr/bin/env python3
"""Ohio WSO records scraper: every named tab is read or the run fails, then one exact-set sync."""

import contextlib
import io
import os
import sys
import unittest
from unittest import mock
from urllib.parse import unquote

import requests

SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import scraper_ohio  # noqa: E402

SHEET_URL = "https://docs.google.com/spreadsheets/d/1fX-Ft3PuLn8BCE2thhwPEXFTEUTN7yJGxWi7LMajAD8/view?gid=0#gid=0"

# The first two classes of the Youth Men, Senior Men and Masters Men tabs as
# gviz serves them, trailing blank columns cut. The merged title row carries
# the first age group and class.
YOUTH_CSV = """\
"Ohio WSO Weightlifting Records (Updated 8/17/2026) Lift 13 and Under 32 kg","Athlete ","Team ","Weight ","Date ","Meet ","Location "
"Snatch","Wilder Gibbs","Clean Slate Weightlifting","14","8/16/2026","2026 Ohio WSO Championships","Columbus, OH"
"Clean & Jerk","Wilder Gibbs","Clean Slate Weightlifting","14","8/16/2026","2026 Ohio WSO Championships","Columbus, OH"
"Total","Wilder Gibbs","Clean Slate Weightlifting","28","8/16/2026","2026 Ohio WSO Championships","Columbus, OH"
"36 kg"
"Snatch","Trae Dauch","Sandusky Weightlifting","38","6/20/2026","2026 USAW Youth Nationals","Colorado Springs, CO"
"Clean & Jerk","Trae Dauch","Sandusky Weightlifting","49","6/20/2026","2026 USAW Youth Nationals","Colorado Springs, CO"
"Total","Trae Dauch","Sandusky Weightlifting","87","6/20/2026","2026 USAW Youth Nationals","Colorado Springs, CO"
"""
SENIOR_CSV = """\
"Ohio WSO Weightlifting Records (Updated 10/07/2026) Lift 60 kg","Athlete ","Team ","Weight ","Date ","Meet ","Location "
"Snatch","WSO Standard","","88","8/1/2026"
"Clean & Jerk","WSO Standard","","122","8/1/2026"
"Total","WSO Standard","","212","8/1/2026"
"65 kg"
"Snatch","WSO Standard","","92","8/1/2026"
"Clean & Jerk","WSO Standard","","124","8/1/2026"
"Total","WSO Standard","","212","8/1/2026"
"""
MASTERS_CSV = """\
"Ohio WSO Weightlifting Records (Updated 8/17/2026) Lift 35 - 39 60 kg","Athlete ","Team ","Weight ","Date ","Meet ","Location "
"Snatch","Derick Puff","Team Aita","78","8/16/2026","2026 Ohio WSO Championships","Columbus, OH"
"Clean & Jerk","","","88","8/1/2026"
"Total","Derick Puff","Team Aita","164","12/7/2025","2025 UMWF World Championships","Daytona Beach, FL"
"65 kg"
"Snatch","WSO Standard","","74","8/1/2026"
"Clean & Jerk","WSO Standard","","94","8/1/2026"
"Total","WSO Standard","","168","8/1/2026"
"""
BY_CATEGORY = {"Youth": YOUTH_CSV, "Junior": SENIOR_CSV, "Senior": SENIOR_CSV, "Masters": MASTERS_CSV}


def tab_csv(tab: str) -> str:
    """The fixture for a tab, its title naming the tab: live tabs never serve identical CSV."""
    text = BY_CATEGORY[tab.split()[0]]
    return text.replace("Ohio WSO Weightlifting Records", f"Ohio WSO Weightlifting Records {tab}", 1)


class FakeResponse:
    def __init__(self, text: str, status_code: int = 200):
        self.text = text
        self.status_code = status_code

    def raise_for_status(self):
        if self.status_code != 200:
            raise requests.HTTPError(f"{self.status_code} Server Error")


def serve(overrides=None):
    """A requests.get stand-in: each tab serves its fixture unless its name is overridden."""
    overrides = overrides or {}
    requested = []

    def get(url, timeout=None):
        tab = unquote(url.rsplit("sheet=", 1)[1])
        requested.append(tab)
        return overrides.get(tab, FakeResponse(tab_csv(tab)))

    get.requested = requested
    return get


def quietly(fn, *args, **kwargs):
    with contextlib.redirect_stdout(io.StringIO()):
        return fn(*args, **kwargs)


class OhioScraperTests(unittest.TestCase):
    def setUp(self):
        self.scraper = scraper_ohio.WSORecordsScraper("Ohio", SHEET_URL)

    def scrape(self, get):
        with mock.patch.object(scraper_ohio.requests, "get", side_effect=get):
            return quietly(self.scraper.scrape_sheet)

    def test_reads_exactly_the_named_tabs(self):
        get = serve()
        records = self.scrape(get)

        self.assertEqual(get.requested, list(scraper_ohio.OHIO_TABS))
        self.assertEqual(len(records), 2 * len(scraper_ohio.OHIO_TABS))
        self.assertEqual(
            {r["age_category"] for r in records}, {"U13", "Junior", "Senior", "Masters 35"}
        )
        masters_men_60 = next(
            r for r in records
            if (r["age_category"], r["gender"], r["weight_class"]) == ("Masters 35", "Men", "60")
        )
        self.assertEqual(
            (masters_men_60["snatch_record"], masters_men_60["cj_record"], masters_men_60["total_record"]),
            (78, 88, 164),
        )

    def test_renamed_tab_raises(self):
        # gviz answers an unknown sheet name with the first tab.
        first_tab = FakeResponse(tab_csv("Youth Women"))
        with self.assertRaisesRegex(ValueError, "'Masters Men' returned the same CSV as 'Youth Women'"):
            self.scrape(serve({"Masters Men": first_tab}))

    def test_failing_tab_raises(self):
        with self.assertRaisesRegex(requests.HTTPError, "500"):
            self.scrape(serve({"Junior Women": FakeResponse("", status_code=500)}))

    def test_empty_tab_raises(self):
        flat = '"Weight Class","Lift","Record","Athlete"\n"60","Snatch","88","WSO Standard"\n'
        with self.assertRaisesRegex(ValueError, "Senior Men"):
            self.scrape(serve({"Senior Men": FakeResponse(flat)}))

    def test_run_syncs_once_with_every_record(self):
        argv = ["scraper_ohio.py", "--wso", "Ohio", "--sheet-url", SHEET_URL, "--allow-shrink"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.dict(os.environ, {"SLACK_WEBHOOK_URL": ""}), \
                mock.patch.object(scraper_ohio.requests, "get", side_effect=serve()), \
                mock.patch.object(scraper_ohio, "sync_wso_records") as sync:
            quietly(scraper_ohio.main)

        sync.assert_called_once()
        (wso, records), kwargs = sync.call_args
        self.assertEqual(wso, "Ohio")
        self.assertEqual(len(records), 2 * len(scraper_ohio.OHIO_TABS))
        self.assertEqual(kwargs, {"dry_run": False, "allow_shrink": True})

    def test_dry_run_touches_no_database(self):
        argv = ["scraper_ohio.py", "--wso", "Ohio", "--sheet-url", SHEET_URL, "--dry-run"]
        with mock.patch.object(sys, "argv", argv), \
                mock.patch.object(scraper_ohio.requests, "get", side_effect=serve()), \
                mock.patch("common.postgres_ingest.IngestClient") as ingest_client:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                scraper_ohio.main()

        ingest_client.assert_not_called()
        self.assertIn("Dry run: would sync 16 Ohio classes", output.getvalue())


if __name__ == "__main__":
    unittest.main()
