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
RECORDS_HEADING = re.compile(
    r"<h[1-6][^>]*>\s*Illinois State Records\s*</h[1-6]>", re.IGNORECASE
)
FILE_DATE = re.compile(r"(20\d{6})[^/]*\.pdf$", re.IGNORECASE)


def find_pdf_href(page_html: str) -> str:
    """The records PDF: the "View (the) Records" button in the page section
    (the <section>, or the rest of the page after the heading if there is
    none) that holds the "Illinois State Records" heading; the newest by the
    date in its file name if there are several. The same words open a banner
    in another section ("Illinois State Records are updated!"), so the heading
    is the heading element, or else the words' last mention. No button there
    fails rather than guessing at the page's other PDFs. Same rule as
    meetcal-app's convex/scrapers/parse/wso/illinois.ts.
    """
    heading_match = RECORDS_HEADING.search(page_html)
    heading = heading_match.start() if heading_match else page_html.rfind("Illinois State Records")
    if heading == -1:
        raise ValueError("Could not find the Illinois State Records section on the page")
    opening = page_html.rfind("<section", 0, heading)
    closing = page_html.find("</section>", heading)
    section = page_html[
        heading if opening == -1 else opening : None if closing == -1 else closing
    ]
    hrefs = [match.group(1) for match in VIEW_RECORDS_LINK.finditer(section)]
    if not hrefs:
        raise ValueError("Could not find the Illinois records PDF URL on the page")

    def date_of(href: str) -> str:
        match = FILE_DATE.search(href)
        return match.group(1) if match else ""

    newest = hrefs[0]
    for href in hrefs[1:]:
        if date_of(href) > date_of(newest):
            newest = href
    return newest


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
