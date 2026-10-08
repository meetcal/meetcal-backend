#!/usr/bin/env python3
"""DMV scraper: the flat layout under DMV's column names.

Run: cd scrapers && PYTHONPATH=. python -m unittest usaw/wso_sheets_scraper/test_files/test_dmv.py
"""

import os
import sys
import unittest
from unittest import mock

SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import scraper_dmv
from scraper_dmv import WSORecordsDMVScraper

DMV_URL = (
    "https://docs.google.com/spreadsheets/d/1vYD2H6si9FyEO-Tc24DoFZOmST0r5hCn"
    "/edit?gid=799684986#gid=799684986"
)

# DMV's gid=799684986 tab.
DMV_TAB = "\n".join(
    [
        '"federation","recordName","Age Group","Gender","ageMin","ageMax","bodyWeightMin","Weight Class","Lift","Record","Name","Club","Date","Event",""',
        '"DMV","DMV","U13","F","0","13","0","36","Snatch","35","STANDARD","","06.01.2025","",""',
        '"DMV","DMV","U13","F","0","13","0","36","Clean & Jerk","41","STANDARD","","06.01.2025","",""',
        '"DMV","DMV","U13","F","0","13","0","36","Total","76","STANDARD","","06.01.2025","",""',
        '"DMV","DMV","U13","F","0","13","63",">63","Snatch","49","STANDARD","","06.01.2025","",""',
        '"DMV","DMV","U13","F","0","13","63",">63","Clean & Jerk","62","STANDARD","","06.01.2025","",""',
        '"DMV","DMV","U13","F","0","13","63",">63","Total","111","STANDARD","","06.01.2025","",""',
    ]
)


def response(text, status_code=200):
    return mock.Mock(status_code=status_code, text=text)


class DMVScraperTests(unittest.TestCase):
    def setUp(self):
        self.scraper = WSORecordsDMVScraper("DMV", DMV_URL)

    def scrape(self, text):
        with mock.patch.object(scraper_dmv.requests, "get", return_value=response(text)) as get:
            return self.scraper.scrape_sheet(), get

    def test_reads_the_tab_the_url_points_at(self):
        _, get = self.scrape(DMV_TAB)

        self.assertEqual(
            get.call_args.args[0],
            "https://docs.google.com/spreadsheets/d/1vYD2H6si9FyEO-Tc24DoFZOmST0r5hCn"
            "/gviz/tq?tqx=out:csv&gid=799684986",
        )

    def test_parses_dmv_columns(self):
        records, _ = self.scrape(DMV_TAB)

        self.assertEqual(
            records,
            [
                {
                    "wso": "DMV",
                    "age_category": "U13",
                    "gender": "Women",
                    "weight_class": "36",
                    "snatch_record": 35,
                    "cj_record": 41,
                    "total_record": 76,
                },
                {
                    "wso": "DMV",
                    "age_category": "U13",
                    "gender": "Women",
                    "weight_class": "63+",
                    "snatch_record": 49,
                    "cj_record": 62,
                    "total_record": 111,
                },
            ],
        )

    def test_run_syncs_every_record_once(self):
        with mock.patch.object(
            scraper_dmv.requests, "get", return_value=response(DMV_TAB)
        ), mock.patch.object(scraper_dmv, "sync_wso_records") as sync, mock.patch("builtins.print"):
            self.scraper.run()

        sync.assert_called_once()
        wso, records = sync.call_args.args
        self.assertEqual(wso, "DMV")
        self.assertEqual(len(records), 2)
        self.assertEqual(sync.call_args.kwargs, {"dry_run": False, "allow_shrink": False})

    def test_zero_records_fail_the_run(self):
        header_only = DMV_TAB.splitlines()[0]
        with mock.patch.object(
            scraper_dmv.requests, "get", return_value=response(header_only)
        ), mock.patch("common.postgres_ingest.IngestClient") as client, mock.patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "parsed 0 records"):
                self.scraper.run()

        client.assert_not_called()

    def test_dry_run_cli_does_no_database_work(self):
        argv = ["scraper_dmv.py", "--wso", "DMV", "--sheet-url", DMV_URL, "--dry-run", "--allow-shrink"]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(
            scraper_dmv.requests, "get", return_value=response(DMV_TAB)
        ), mock.patch("common.postgres_ingest.IngestClient") as client, mock.patch(
            "builtins.print"
        ) as printed:
            scraper_dmv.main()

        client.assert_not_called()
        output = "\n".join(str(call.args[0]) for call in printed.call_args_list if call.args)
        self.assertIn("Dry run: would sync 2 DMV classes", output)


if __name__ == "__main__":
    unittest.main()
