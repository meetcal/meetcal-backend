#!/usr/bin/env python3
"""California South scraper.

Run: cd scrapers && PYTHONPATH=. python -m unittest usaw/wso_sheets_scraper/test_files/test_california_south.py
"""

import os
import sys
import unittest
from unittest import mock

SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import scraper_california_south_auto as california_south
from scraper_california_south_auto import WSORecordsCaliforniaSouthScraper

SHEET_URL = (
    "https://docs.google.com/spreadsheets/d/1PHYJ-lhkXYMrQIIo6YaipePFxruSfbRw1TEUtIoknR0"
    "/edit?usp=sharing"
)

HEADER = '"federation","recordName","ageGroup","gender","ageMin","ageMax","bodyWeightMin","bodyWeightMax","lift","American Record","WSO record","name","date","place"'

# Rows from the live sheet's first tab.
U11_WOMEN = [
    '"California South","California South","U11","F","0","11","0","30","Snatch","22","15","STANDARD","2025-06-01",""',
    '"California South","California South","U11","F","0","11","0","30","Clean & Jerk","32","22","STANDARD","2025-06-01",""',
    '"California South","California South","U11","F","0","11","0","30","Total","54","37","STANDARD","2025-06-01",""',
    '"California South","California South","U11","F","0","11","30","33","Snatch","40","28","STANDARD","2025-06-01",""',
    '"California South","California South","U11","F","0","11","30","33","Clean & Jerk","50","35","STANDARD","2025-06-01",""',
    '"California South","California South","U11","F","0","11","30","33","Total","90","63","STANDARD","2025-06-01",""',
    '"California South","California South","U11","F","0","11","61","","Snatch","50","35","STANDARD","2026-08-01",""',
    '"California South","California South","U11","F","0","11","61","","Clean & Jerk","67","46","STANDARD","2026-08-01",""',
    '"California South","California South","U11","F","0","11","61","","Total","117","81","STANDARD","2026-08-01",""',
]
JUNIOR_MEN = [
    '"California South","California South","JR","M","15","20","60","65","Snatch","125","87","STANDARD","2025-06-01",""',
    '"California South","California South","JR","M","15","20","60","65","Clean & Jerk","165","115","STANDARD","2025-06-01",""',
    '"California South","California South","JR","M","15","20","60","65","Total","292","204","STANDARD","2025-06-01",""',
    '"California South","California South","JR","M","15","20","110","","Snatch","155","108","STANDARD","2025-06-01",""',
]


def sheet(*rows):
    return "\n".join([HEADER, *rows])


def response(text, status_code=200):
    return mock.Mock(status_code=status_code, text=text)


class CaliforniaSouthParseTests(unittest.TestCase):
    def setUp(self):
        self.scraper = WSORecordsCaliforniaSouthScraper("California South", SHEET_URL)

    def scrape(self, text):
        with mock.patch.object(
            california_south.requests, "get", return_value=response(text)
        ) as get:
            return self.scraper.scrape_sheet(), get

    def test_reads_the_first_tab_of_the_configured_sheet(self):
        _, get = self.scrape(sheet(*U11_WOMEN))

        self.assertEqual(
            get.call_args.args[0],
            "https://docs.google.com/spreadsheets/d/1PHYJ-lhkXYMrQIIo6YaipePFxruSfbRw1TEUtIoknR0"
            "/gviz/tq?tqx=out:csv",
        )

    def test_names_classes_by_their_maximum(self):
        records, _ = self.scrape(sheet(*U11_WOMEN))

        # 30-33 is "33", not "30" (the lower bound), so it no longer collides
        # with 0-30; the open top class is "<min>+". Values are "WSO record".
        self.assertEqual(
            [(r["weight_class"], r["snatch_record"], r["cj_record"], r["total_record"]) for r in records],
            [("30", 15, 22, 37), ("33", 28, 35, 63), ("61+", 35, 46, 81)],
        )
        self.assertEqual({(r["age_category"], r["gender"], r["wso"]) for r in records}, {("U11", "Women", "California South")})

    def test_normalizes_age_groups(self):
        records, _ = self.scrape(sheet(*JUNIOR_MEN))

        self.assertEqual(
            records,
            [
                {
                    "wso": "California South",
                    "age_category": "Junior",
                    "gender": "Men",
                    "weight_class": "65",
                    "snatch_record": 87,
                    "cj_record": 115,
                    "total_record": 204,
                },
                {
                    "wso": "California South",
                    "age_category": "Junior",
                    "gender": "Men",
                    "weight_class": "110+",
                    "snatch_record": 108,
                    "cj_record": None,
                    "total_record": None,
                },
            ],
        )

    def test_skips_adaptive_groups_and_unknown_lifts(self):
        records, _ = self.scrape(
            sheet(
                '"California South","California South","JR ADAP","M","15","20","60","65","Snatch","125","87","STANDARD","2025-06-01",""',
                '"California South","California South","U11","F","0","11","0","30","Best Lift","22","15","STANDARD","2025-06-01",""',
            )
        )

        self.assertEqual(records, [])


class CaliforniaSouthRunTests(unittest.TestCase):
    def setUp(self):
        self.scraper = WSORecordsCaliforniaSouthScraper("California South", SHEET_URL)

    def test_run_syncs_every_record_once(self):
        with mock.patch.object(
            california_south.requests, "get", return_value=response(sheet(*U11_WOMEN, *JUNIOR_MEN))
        ), mock.patch.object(california_south, "sync_wso_records") as sync, mock.patch("builtins.print"):
            self.scraper.run(allow_shrink=True)

        sync.assert_called_once()
        wso, records = sync.call_args.args
        self.assertEqual(wso, "California South")
        self.assertEqual(len(records), 5)
        self.assertEqual(sync.call_args.kwargs, {"dry_run": False, "allow_shrink": True})

    def test_zero_records_fail_the_run(self):
        with mock.patch.object(
            california_south.requests, "get", return_value=response(HEADER)
        ), mock.patch("common.postgres_ingest.IngestClient") as client, mock.patch("builtins.print"):
            with self.assertRaisesRegex(ValueError, "parsed 0 records"):
                self.scraper.run()

        client.assert_not_called()

    def test_dry_run_cli_does_no_database_work(self):
        argv = ["scraper_california_south_auto.py", "--wso", "California South", "--sheet-url", SHEET_URL, "--dry-run"]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(
            california_south.requests, "get", return_value=response(sheet(*U11_WOMEN))
        ), mock.patch("common.postgres_ingest.IngestClient") as client, mock.patch.object(
            california_south, "load_dotenv"
        ) as load_dotenv, mock.patch("builtins.print") as printed:
            california_south.main()

        client.assert_not_called()
        load_dotenv.assert_not_called()
        output = "\n".join(str(call.args[0]) for call in printed.call_args_list if call.args)
        self.assertIn("Dry run: would sync 3 California South classes", output)
        self.assertIn("U11 | Women | 33: snatch=28", output)


if __name__ == "__main__":
    unittest.main()
