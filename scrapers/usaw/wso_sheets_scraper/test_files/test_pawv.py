#!/usr/bin/env python3

import os
import sys
import unittest
from unittest.mock import patch


SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import scraper_pawv  # noqa: E402
from scraper_pawv import WSORecordsPAWVScraper  # noqa: E402

SHEET_ID = "2PACX-1vR8exp9-mwi8dpkZa9-48G-CUVuZ5rAlpOYdMCiNMka25wZ6V2XPLurpgMDtyiarqnQxYrW6dWfQ042"

# The live Masters Men tab (gid 14757518, October 2026) as its CSV export
# reads, cut to the two heaviest classes of 35-39 and the first of 40-44.
MASTERS_MEN_CSV = "\r\n".join([
    "Lift,Name,Team,Weight,Date,Meet,Location",
    "Men's Masters (35-39),,,,,,",
    "110kg,,,,,,",
    'Snatch,Thomas Duer,EAST COAST GOLD W/L TEAM,148,2025-12-07,2025 Virus Weightlifting Finals & UMWF World Championships,"Daytona Beach, FL"',
    'Clean & Jerk,Thomas Duer,EAST COAST GOLD W/L TEAM,177,2025-12-07,2025 Virus Weightlifting Finals & UMWF World Championships,"Daytona Beach, FL"',
    'Total,Thomas Duer,EAST COAST GOLD W/L TEAM,325,2025-12-07,2025 Virus Weightlifting Finals & UMWF World Championships,"Daytona Beach, FL"',
    "+110kg,,,,,,",
    'Snatch,Dimitri Albury,Rising Tide Weightlifting,152,2026-03-06,2026 Virus Weigthlifting Series 1,"Columbus, OH"',
    'Clean & Jerk,Dimitri Albury,Rising Tide Weightlifting,210,2026-05-09,2026 Never Give Up Spring Classic,"West Chester, PA"',
    'Total,Dimitri Albury,Rising Tide Weightlifting,358,2026-03-06,2026 Virus Weigthlifting Series 1,"Columbus, OH"',
    "Men's Masters (40-44),,,,,,",
    "60kg,,,,,,",
    "Snatch,STANDARD,,73,2025-06-01,,",
    "Clean & Jerk,STANDARD,,90,2025-06-01,,",
    "Total,STANDARD,,163,2025-06-01,,",
])


def classes(records):
    return [
        (r["age_category"], r["gender"], r["weight_class"], r["snatch_record"], r["cj_record"], r["total_record"])
        for r in records
    ]


class PAWVParserTests(unittest.TestCase):
    def setUp(self):
        self.scraper = WSORecordsPAWVScraper("Pennsylvania-West Virginia", SHEET_ID)

    def test_keeps_the_heaviest_class_before_each_section_header(self):
        # A section header used to reset the class in progress without saving
        # it, so 35-39's +110kg (each section's heaviest but the tab's last)
        # was never written.
        records = self.scraper.parse_tab(MASTERS_MEN_CSV, "Men", "Masters")

        self.assertEqual(
            classes(records),
            [
                ("Masters 35", "Men", "110", 148, 177, 325),
                ("Masters 35", "Men", "110+", 152, 210, 358),
                ("Masters 40", "Men", "60", 73, 90, 163),
            ],
        )

    def test_a_tab_parsed_to_nothing_fails_the_whole_read(self):
        def fetch(gid):
            return "Lift,Name,Team,Weight,Date,Meet,Location" if gid == "846901037" else MASTERS_MEN_CSV

        with patch.object(self.scraper, "fetch_csv_data", side_effect=fetch):
            with self.assertRaisesRegex(ValueError, "gid=846901037"):
                self.scraper.scrape_all_tabs()


class PAWVRunTests(unittest.TestCase):
    def setUp(self):
        self.scraper = WSORecordsPAWVScraper("Pennsylvania-West Virginia", SHEET_ID)
        self.records = self.scraper.parse_tab(MASTERS_MEN_CSV, "Men", "Masters")

    def test_run_syncs_every_tab_once(self):
        result = {"inserted": 0, "updated": 0, "deleted": 0, "unchanged": 3}
        with patch.object(self.scraper, "scrape_all_tabs", return_value=self.records), \
                patch.object(scraper_pawv, "sync_wso_records", return_value=result) as sync:
            self.scraper.run(allow_shrink=True)
        sync.assert_called_once_with(
            "Pennsylvania-West Virginia", self.records, dry_run=False, allow_shrink=True
        )

    def test_dry_run_flag_does_no_database_work(self):
        argv = ["scraper_pawv.py", "--wso", "Pennsylvania-West Virginia", "--sheet-id", SHEET_ID, "--dry-run"]
        with patch.object(sys, "argv", argv), \
                patch.object(scraper_pawv, "load_dotenv"), \
                patch.object(WSORecordsPAWVScraper, "scrape_all_tabs", return_value=self.records), \
                patch("common.postgres_ingest.IngestClient") as ingest:
            scraper_pawv.main()
        ingest.assert_not_called()


if __name__ == "__main__":
    unittest.main()
