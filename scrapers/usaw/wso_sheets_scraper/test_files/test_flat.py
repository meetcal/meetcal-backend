#!/usr/bin/env python3
"""Flat-format scraper (Georgia, Pacific Northwest, California North).

Run: cd scrapers && PYTHONPATH=. python -m unittest usaw/wso_sheets_scraper/test_files/test_flat.py
"""

import os
import sys
import unittest
from unittest import mock

SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import scraper_ga_pnw
from scraper_ga_pnw import WSORecordsFlatScraper

CALIFORNIA_NORTH_URL = (
    "https://docs.google.com/spreadsheets/d/1ZAs27jQCPYTVgLuQ-feBHSO-BgGjGCewUs0djG23pXQ"
    "/edit?gid=35344992#gid=35344992"
)

# California North's gid=35344992 tab (the flat data), trailing blank columns cut.
CALIFORNIA_NORTH_GID_TAB = "\n".join(
    [
        '"startDate","endDate","federation","recordName","ageGroup","gender","ageMin","ageMax","bodyWeightMin","bodyWeightMax","lift","record","name","date","place"',
        '"2025-05-30","2026-08-01","Norcal","California North Central","U11","F","0","11","0","30","Snatch","20","Avery Gillum","2025-10-18","62nd Don Wilson\'s GOLDEN WEST 2025"',
        '"2025-05-30","2026-08-01","Norcal","California North Central","U11","F","0","11","0","30","Clean & Jerk","30","Avery Gillum","2025-10-18","62nd Don Wilson\'s GOLDEN WEST 2025"',
        '"2025-05-30","2026-08-01","Norcal","California North Central","U11","F","0","11","0","30","Total","50","Avery Gillum","2025-10-18","62nd Don Wilson\'s GOLDEN WEST 2025"',
        '"2025-05-30","2026-08-01","Norcal","California North Central","U11","F","0","11","63",">63","Snatch","52","STANDARD","",""',
        '"2025-05-30","2026-08-01","Norcal","California North Central","U11","F","0","11","63",">63","Clean & Jerk","68","STANDARD","",""',
        '"2025-05-30","2026-08-01","Norcal","California North Central","U11","F","0","11","63",">63","Total","120","STANDARD","",""',
        '"2025-05-30","2026-08-01","Norcal","California North Central","Open","M","0","999","56","60","Snatch","107","STANDARD","",""',
        '"2025-05-30","2026-08-01","Norcal","California North Central","Open","M","0","999","56","60","Clean & Jerk","149","STANDARD","",""',
    ]
)

# California North's "Current Records" tab since mid-2026: a human-readable
# layout with none of the flat columns.
CALIFORNIA_NORTH_NAMED_TAB = "\n".join(
    [
        '"Category","Gender","Age Range","Weight Class","Lift","Weight","Athlete Name","Date Set","Event"',
        '"Under 11","Girls","0 - 11","30kg","Snatch","20 kg","Avery Gillum","2025-10-18","62nd Don Wilson\'s GOLDEN WEST 2025"',
        '"Under 11","Girls","0 - 11","30kg","Clean & Jerk","30 kg","Avery Gillum","2025-10-18","62nd Don Wilson\'s GOLDEN WEST 2025"',
    ]
)


def response(text, status_code=200):
    return mock.Mock(status_code=status_code, text=text)


class FlatSheetTabTests(unittest.TestCase):
    def test_reads_the_tab_the_url_points_at(self):
        scraper = WSORecordsFlatScraper("California North", CALIFORNIA_NORTH_URL)
        with mock.patch.object(
            scraper_ga_pnw.requests, "get", return_value=response(CALIFORNIA_NORTH_GID_TAB)
        ) as get:
            records = scraper.scrape_sheet()

        get.assert_called_once()
        url = get.call_args.args[0]
        self.assertEqual(
            url,
            "https://docs.google.com/spreadsheets/d/1ZAs27jQCPYTVgLuQ-feBHSO-BgGjGCewUs0djG23pXQ"
            "/gviz/tq?tqx=out:csv&gid=35344992",
        )
        self.assertNotIn("sheet=", url)
        self.assertEqual(len(records), 3)

    def test_falls_back_to_the_current_records_tab_without_a_gid(self):
        scraper = WSORecordsFlatScraper(
            "Georgia", "https://docs.google.com/spreadsheets/d/abc123/edit"
        )
        with mock.patch.object(
            scraper_ga_pnw.requests, "get", return_value=response(CALIFORNIA_NORTH_GID_TAB)
        ) as get:
            scraper.scrape_sheet()

        self.assertEqual(
            get.call_args.args[0],
            "https://docs.google.com/spreadsheets/d/abc123/gviz/tq?tqx=out:csv"
            "&sheet=Current%20Records",
        )

    def test_http_error_fails(self):
        scraper = WSORecordsFlatScraper("California North", CALIFORNIA_NORTH_URL)
        with mock.patch.object(scraper_ga_pnw.requests, "get", return_value=response("", 500)):
            with self.assertRaises(Exception):
                scraper.scrape_sheet()


class FlatSheetParseTests(unittest.TestCase):
    def parse(self, text):
        scraper = WSORecordsFlatScraper("California North", CALIFORNIA_NORTH_URL)
        with mock.patch.object(scraper_ga_pnw.requests, "get", return_value=response(text)):
            return scraper.scrape_sheet()

    def test_groups_lifts_by_class_named_by_upper_bound(self):
        records = self.parse(CALIFORNIA_NORTH_GID_TAB)

        self.assertEqual(
            records,
            [
                {
                    "wso": "California North",
                    "age_category": "U11",
                    "gender": "Women",
                    "weight_class": "30",
                    "snatch_record": 20,
                    "cj_record": 30,
                    "total_record": 50,
                },
                {
                    "wso": "California North",
                    "age_category": "U11",
                    "gender": "Women",
                    "weight_class": "63+",
                    "snatch_record": 52,
                    "cj_record": 68,
                    "total_record": 120,
                },
                {
                    "wso": "California North",
                    "age_category": "Senior",
                    "gender": "Men",
                    "weight_class": "60",
                    "snatch_record": 107,
                    "cj_record": 149,
                    "total_record": None,
                },
            ],
        )

    def test_human_readable_layout_parses_to_nothing(self):
        self.assertEqual(self.parse(CALIFORNIA_NORTH_NAMED_TAB), [])


class FlatRunTests(unittest.TestCase):
    def setUp(self):
        self.scraper = WSORecordsFlatScraper("California North", CALIFORNIA_NORTH_URL)

    def test_run_syncs_every_record_once(self):
        with mock.patch.object(
            scraper_ga_pnw.requests, "get", return_value=response(CALIFORNIA_NORTH_GID_TAB)
        ), mock.patch.object(scraper_ga_pnw, "sync_wso_records") as sync, mock.patch(
            "builtins.print"
        ):
            self.scraper.run(allow_shrink=True)

        sync.assert_called_once()
        wso, records = sync.call_args.args
        self.assertEqual(wso, "California North")
        self.assertEqual(len(records), 3)
        self.assertEqual(sync.call_args.kwargs, {"dry_run": False, "allow_shrink": True})

    def test_zero_records_fail_the_run(self):
        with mock.patch.object(
            scraper_ga_pnw.requests, "get", return_value=response(CALIFORNIA_NORTH_NAMED_TAB)
        ), mock.patch("common.postgres_ingest.IngestClient") as client, mock.patch(
            "builtins.print"
        ):
            with self.assertRaisesRegex(ValueError, "parsed 0 records"):
                self.scraper.run()

        client.assert_not_called()

    def test_dry_run_cli_does_no_database_work(self):
        argv = ["scraper_ga_pnw.py", "--wso", "California North", "--sheet-url", CALIFORNIA_NORTH_URL, "--dry-run"]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(
            scraper_ga_pnw.requests, "get", return_value=response(CALIFORNIA_NORTH_GID_TAB)
        ), mock.patch("common.postgres_ingest.IngestClient") as client, mock.patch(
            "builtins.print"
        ) as printed:
            scraper_ga_pnw.main()

        client.assert_not_called()
        output = "\n".join(str(call.args[0]) for call in printed.call_args_list if call.args)
        self.assertIn("Dry run: would sync 3 California North classes", output)
        self.assertIn("U11 | Women | 63+", output)


if __name__ == "__main__":
    unittest.main()
