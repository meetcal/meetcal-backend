#!/usr/bin/env python3

import os
import sys
import unittest
from unittest.mock import MagicMock, patch


SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "manual_scrapers"))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

import scraper_newengland_auto  # noqa: E402
from scraper_newengland_auto import NewEnglandAutoScraper  # noqa: E402
from scraper_pdf_newengland import WSORecordsNewEnglandScraper  # noqa: E402


def blank(weight_class):
    """A class with no record yet, as the U11 PDFs list every one."""
    return [
        [weight_class, "Snatch", "OPEN", "", "", "", ""],
        [None, "C&J", "OPEN", "", "", "", ""],
        [None, "Total", "OPEN", "", "", "", ""],
        ["", None, None, None, None, None, None],
    ]


# Page 1 of the live 11U Youth Men's PDF (October 2026) as pdfplumber's
# extract_tables() reads it, cut to its first and last classes.
U11_MEN_PAGE = [
    [
        ["11U Youth Men's Records", None, None, None, None, None, None],
        ["Class", "Lift", "Name", "Representing", "Location/Meet", "Weight", "Date"],
        *blank("32"),
        *blank("36"),
        *blank("70+"),
    ]
]


def record(weight_class, snatch=50):
    return {
        "wso": "New England",
        "age_category": "Senior",
        "gender": "Men",
        "weight_class": weight_class,
        "snatch_record": snatch,
        "cj_record": None,
        "total_record": None,
    }


class NewEnglandParserTests(unittest.TestCase):
    def setUp(self):
        self.scraper = WSORecordsNewEnglandScraper("New England", "https://example.com/ne.pdf")

    def test_reads_the_11u_page(self):
        # "11U Youth" took the Youth branch, matched no 17/15/13 group and
        # returned no age, so every U11 class was dropped.
        records = self.scraper.parse_tables(U11_MEN_PAGE)

        self.assertEqual(
            [(r["age_category"], r["gender"], r["weight_class"]) for r in records],
            [("U11", "Men", "32"), ("U11", "Men", "36"), ("U11", "Men", "70+")],
        )
        self.assertEqual(
            {(r["snatch_record"], r["cj_record"], r["total_record"]) for r in records},
            {(None, None, None)},
        )

    def test_section_headers(self):
        parse = self.scraper._parse_section_header
        self.assertEqual(parse("11U Youth Women's Records"), ("U11", "Women"))
        self.assertEqual(parse("13U Youth Men's Records"), ("U13", "Men"))
        self.assertEqual(parse("16/17 Youth Men's Records"), ("U17", "Men"))
        self.assertEqual(parse("Open Women's Records"), ("Senior", "Women"))


class FakePdf:
    """Stands in for WSORecordsNewEnglandScraper: one PDF's records by URL."""

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
    {"category": "Open Men's Records", "url": "https://example.com/a.pdf"},
    {"category": "PDF 2", "url": "https://example.com/b.pdf"},
]


class NewEnglandAutoTests(unittest.TestCase):
    def scrape(self, parsed):
        FakePdf.parsed = parsed
        FakePdf.cleaned = []
        scraper = NewEnglandAutoScraper(dry_run=True)
        scraper.EXPECTED_PDFS = len(PDFS)
        with patch.object(scraper_newengland_auto, "WSORecordsNewEnglandScraper", FakePdf), \
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
        scraper = NewEnglandAutoScraper(dry_run=True)
        with patch.object(scraper, "fetch_pdf_urls", return_value=[]):
            with self.assertRaisesRegex(ValueError, "found 0 distinct records PDFs"):
                scraper.scrape_records()

    def test_a_missing_or_repeated_pdf_fails_the_run(self):
        scraper = NewEnglandAutoScraper(dry_run=True)
        scraper.EXPECTED_PDFS = len(PDFS)
        for links in (PDFS[:1], [PDFS[0], PDFS[0]]):
            with patch.object(scraper, "fetch_pdf_urls", return_value=links):
                with self.assertRaisesRegex(ValueError, f"expected {len(PDFS)}"):
                    scraper.scrape_records()

    def test_run_syncs_the_whole_wso_once(self):
        records = [record("60"), record("65")]
        scraper = NewEnglandAutoScraper(allow_shrink=True)
        result = {"inserted": 0, "updated": 0, "deleted": 0, "unchanged": 2}
        with patch.object(scraper, "scrape_records", return_value=records), \
                patch.object(scraper_newengland_auto, "sync_wso_records", return_value=result) as sync:
            scraper.run()
        sync.assert_called_once_with("New England", records, dry_run=False, allow_shrink=True)

    def test_dry_run_flag_does_no_database_work(self):
        records = [record("60")]
        with patch.object(sys, "argv", ["scraper_newengland_auto.py", "--dry-run"]), \
                patch.object(scraper_newengland_auto, "load_dotenv"), \
                patch.object(NewEnglandAutoScraper, "scrape_records", return_value=records), \
                patch("common.postgres_ingest.IngestClient") as ingest:
            scraper_newengland_auto.main()
        ingest.assert_not_called()

    def test_no_flags_syncs_as_the_cron_runs_it(self):
        with patch.object(sys, "argv", ["scraper_newengland_auto.py"]), \
                patch.object(scraper_newengland_auto, "load_dotenv"), \
                patch.object(NewEnglandAutoScraper, "run", MagicMock()) as run, \
                patch.object(NewEnglandAutoScraper, "__init__", return_value=None) as init:
            scraper_newengland_auto.main()
        init.assert_called_once_with(dry_run=False, allow_shrink=False)
        run.assert_called_once_with()


if __name__ == "__main__":
    unittest.main()
