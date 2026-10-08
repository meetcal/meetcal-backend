#!/usr/bin/env python3
"""
Automated scraper for Illinois WSO records.
"""

import argparse
import html
import os
import re
import sys
from urllib.parse import urljoin

import requests
from dotenv import load_dotenv

sys.path.insert(
    0,
    os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "manual_scrapers"
    ),
)

from scraper_pdf_illinois import WSORecordsIllinoisScraper


VIEW_RECORDS_LINK = re.compile(
    r'<a[^>]+href="([^"]+\.pdf)"[^>]*>\s*View(?:\s+the)?\s+Records\s*</a>', re.IGNORECASE
)
PDF_LINK = re.compile(r'href="([^"]+\.pdf)"', re.IGNORECASE)
RECORDS_FILE = re.compile(
    r"(?:IL[-_ ]?WSO[-_ ]?Records|Illinois[-_ ]?State[-_ ]?Records)[^/]*\.pdf$", re.IGNORECASE
)


def find_pdf_href(page_html: str) -> str:
    """The records PDF the page links: its "View (the) Records" button, the
    one whose file is named for the records if there are several, else the
    first after the "Illinois State Records" heading. That text also opens a
    banner further up ("Illinois State Records are updated!"), so the button
    can't be looked for only within a stretch after its first mention.
    """
    buttons = [(m.start(), m.group(1)) for m in VIEW_RECORDS_LINK.finditer(page_html)]
    heading = page_html.find("Illinois State Records")
    for _, href in buttons:
        if RECORDS_FILE.search(href):
            return href
    for at, href in buttons:
        if at > heading:
            return href
    for match in PDF_LINK.finditer(page_html):
        if RECORDS_FILE.search(match.group(1)):
            return match.group(1)
    raise ValueError("Could not find the Illinois records PDF URL on the page")


class IllinoisAutoScraper:
    def __init__(self, dry_run: bool = False):
        self.records_page_url = "https://www.illinoisweightlifting.com/"
        self.wso_name = "Illinois"
        self.dry_run = dry_run

    def fetch_pdf_url(self) -> str:
        print(f"Fetching records page: {self.records_page_url}")
        response = requests.get(
            self.records_page_url,
            headers={
                "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
            },
            timeout=30,
        )
        response.raise_for_status()

        pdf_url = urljoin(self.records_page_url, find_pdf_href(html.unescape(response.text)))
        print(f"Found Illinois records PDF: {pdf_url}")
        return pdf_url

    def run(self):
        print("=" * 80)
        print(f"ILLINOIS WSO - AUTOMATED SCRAPER{' (DRY RUN)' if self.dry_run else ''}")
        print("=" * 80)
        print()

        pdf_url = self.fetch_pdf_url()
        scraper = WSORecordsIllinoisScraper(self.wso_name, pdf_url)
        scraper.run(dry_run=self.dry_run)


def main():
    parser = argparse.ArgumentParser(description="Automated scraper for Illinois WSO records")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--dry-run",
        action="store_true",
        help="Parse without updating Postgres (also the default)",
    )
    mode.add_argument(
        "--apply",
        action="store_true",
        help="Replace the Illinois Postgres records with the parsed PDF records",
    )
    args = parser.parse_args()

    load_dotenv()

    if not args.apply:
        print("Illinois database write approval gate is active; running dry-run only")
    scraper = IllinoisAutoScraper(dry_run=not args.apply)
    scraper.run()


if __name__ == "__main__":
    main()
