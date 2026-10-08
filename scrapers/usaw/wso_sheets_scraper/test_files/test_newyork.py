#!/usr/bin/env python3

import os
import sys
import unittest
from unittest.mock import MagicMock, patch


SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "manual_scrapers"))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import scraper_newyork_auto  # noqa: E402
from scraper_newyork_auto import NewYorkAutoScraper  # noqa: E402
from scraper_pdf_newyork import WSORecordsNewYorkScraper  # noqa: E402


def record(weight_class, snatch=50):
    return {
        "wso": "New York",
        "age_category": "Senior",
        "gender": "Men",
        "weight_class": weight_class,
        "snatch_record": snatch,
        "cj_record": None,
        "total_record": None,
    }


# The top of page 1 of the live Youth PDF (October 2026) as pdfplumber's
# extract_tables() reads it, cut to two classes.
YOUTH_PAGE = [
    [
        ["New York State Records", None, None, None, None, None],
        ["Youth Men", None, None, None, None, None],
        ["Wt. Class", "Lift", "Record", "Name", "Date", "Event"],
        ["55", "Snatch", "46 kg", "Record Standard", "-", "-"],
        [None, "Clean and Jerk", "57 kg", "Record Standard", "-", "-"],
        [None, "Total", "103 kg", "Record Standard", "-", "-"],
        ["", None, None, None, None, None],
        ["65", "Snatch", "77 kg", "Caden Vanderhoof", "9/28/2025", "NYS Championships"],
        [None, "Clean and Jerk", "88 kg", "Aaron Li", "9/27/2026", "NYS Championships"],
        [None, "Total", "163 kg", "Caden Vanderhoof", "9/28/2025", "NYS Championships"],
    ]
]


class NewYorkParserTests(unittest.TestCase):
    def test_reads_a_youth_table(self):
        scraper = WSORecordsNewYorkScraper("New York", "https://example.com/youth.pdf")
        records = scraper.parse_tables(YOUTH_PAGE)

        self.assertEqual(
            [(r["age_category"], r["gender"], r["weight_class"], r["snatch_record"], r["cj_record"], r["total_record"]) for r in records],
            [("Youth", "Men", "55", 46, 57, 103), ("Youth", "Men", "65", 77, 88, 163)],
        )


class FakePdf:
    """Stands in for WSORecordsNewYorkScraper: one PDF's records by URL."""

    parsed = {}
    cleaned = []

    def __init__(self, wso_name, pdf_url):
        self.pdf_url = pdf_url

    def download_pdf(self):
        if self.parsed[self.pdf_url] is None:
            raise OSError("download failed")

    def scrape_pdf(self):
        return self.parsed[self.pdf_url]

    def cleanup(self):
        FakePdf.cleaned.append(self.pdf_url)


PDFS = [
    {"category": "A", "url": "https://example.com/a.pdf"},
    {"category": "B", "url": "https://example.com/b.pdf"},
]


class NewYorkAutoTests(unittest.TestCase):
    def scrape(self, parsed):
        FakePdf.parsed = parsed
        FakePdf.cleaned = []
        scraper = NewYorkAutoScraper(dry_run=True)
        with patch.object(scraper_newyork_auto, "WSORecordsNewYorkScraper", FakePdf), \
                patch.object(scraper, "fetch_pdf_urls", return_value=PDFS):
            return scraper.scrape_records()

    def test_returns_every_pdfs_records(self):
        records = self.scrape({PDFS[0]["url"]: [record("60")], PDFS[1]["url"]: [record("65")]})
        self.assertEqual([r["weight_class"] for r in records], ["60", "65"])
        self.assertEqual(FakePdf.cleaned, [PDFS[0]["url"], PDFS[1]["url"]])

    def test_a_pdf_parsed_to_nothing_fails_the_run(self):
        with self.assertRaisesRegex(ValueError, "b.pdf"):
            self.scrape({PDFS[0]["url"]: [record("60")], PDFS[1]["url"]: []})

    def test_a_failed_download_fails_the_run_and_is_cleaned_up(self):
        with self.assertRaises(OSError):
            self.scrape({PDFS[0]["url"]: None, PDFS[1]["url"]: [record("65")]})
        self.assertEqual(FakePdf.cleaned, [PDFS[0]["url"]])

    def test_no_pdfs_fails_the_run(self):
        scraper = NewYorkAutoScraper(dry_run=True)
        with patch.object(scraper, "fetch_pdf_urls", return_value=[]):
            with self.assertRaisesRegex(ValueError, "no records PDFs"):
                scraper.scrape_records()

    def test_run_syncs_the_whole_wso_once(self):
        records = [record("60"), record("65")]
        scraper = NewYorkAutoScraper(allow_shrink=True)
        result = {"inserted": 0, "updated": 0, "deleted": 0, "unchanged": 2}
        with patch.object(scraper, "scrape_records", return_value=records), \
                patch.object(scraper_newyork_auto, "sync_wso_records", return_value=result) as sync:
            scraper.run()
        sync.assert_called_once_with("New York", records, dry_run=False, allow_shrink=True)

    def test_dry_run_flag_does_no_database_work(self):
        records = [record("60")]
        with patch.object(sys, "argv", ["scraper_newyork_auto.py", "--dry-run"]), \
                patch.object(scraper_newyork_auto, "load_dotenv"), \
                patch.object(NewYorkAutoScraper, "scrape_records", return_value=records), \
                patch("common.postgres_ingest.IngestClient") as ingest:
            scraper_newyork_auto.main()
        ingest.assert_not_called()

    def test_no_flags_syncs_as_the_cron_runs_it(self):
        with patch.object(sys, "argv", ["scraper_newyork_auto.py"]), \
                patch.object(scraper_newyork_auto, "load_dotenv"), \
                patch.object(NewYorkAutoScraper, "run", MagicMock()) as run, \
                patch.object(NewYorkAutoScraper, "__init__", return_value=None) as init:
            scraper_newyork_auto.main()
        init.assert_called_once_with(dry_run=False, allow_shrink=False)
        run.assert_called_once_with()


if __name__ == "__main__":
    unittest.main()
