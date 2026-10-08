#!/usr/bin/env python3
"""
New England WSO Records - Automated Multi-PDF Scraper

This script automatically scrapes the New England WSO records page,
extracts all PDF URLs, and processes them using the PDF scraper.

Every PDF is read before anything is written: Postgres gets the WSO's whole
set in one exact-set sync (``utils.sync_wso_records``), so a PDF that fails
to download or parses to nothing fails the run instead of deleting its
classes.

USAGE:
  Dry-run (test without making changes):
    source venv/bin/activate && python scraper_newengland_auto.py --dry-run

  Live run (replace the WSO's Postgres records with every PDF's records):
    source venv/bin/activate && python scraper_newengland_auto.py
"""

import argparse
import os
import re

# Import the PDF scraper
import sys
from typing import Any, Dict, List

import requests
from dotenv import load_dotenv

SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "manual_scrapers"))
sys.path.insert(0, SCRAPER_DIR)
from utils import every_part, sync_wso_records  # noqa: E402
from scraper_pdf_newengland import WSORecordsNewEnglandScraper  # noqa: E402


class NewEnglandAutoScraper:
    """Automated scraper that fetches all PDF URLs and processes them."""

    # The records PDFs the page links (a men's and a women's PDF each for U11, youth, junior and senior/masters). The sync is an exact set, so
    # a run that finds a different number fails rather than syncing a WSO
    # with a PDF missing; after checking the page by hand, update this.
    EXPECTED_PDFS = 8

    def __init__(self, dry_run: bool = False, allow_shrink: bool = False):
        """
        Initialize auto scraper.

        Args:
            dry_run: If True, parse and print without touching Postgres
            allow_shrink: Let the sync delete more than a quarter of the stored classes
        """
        self.records_page_url = "https://www.newenglandweightlifting.com/records"
        self.wso_name = "New England"
        self.dry_run = dry_run
        self.allow_shrink = allow_shrink
        self.slack_webhook_url = os.getenv("SLACK_WEBHOOK_URL")

    def fetch_pdf_urls(self) -> List[Dict[str, str]]:
        """
        Fetch the records page and extract all PDF URLs.

        Returns:
            List of dicts with 'category' and 'url' keys
        """
        print(f"Fetching records page: {self.records_page_url}")

        headers = {
            "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
        }

        response = requests.get(self.records_page_url, headers=headers, timeout=30)
        response.raise_for_status()

        html_content = response.text

        # Extract all PDF URLs from the page
        # Look for links ending in .pdf
        pdf_pattern = r'href="(https://www\.newenglandweightlifting\.com/_files/ugd/[a-zA-Z0-9_/]+\.pdf)"'
        pdf_urls = re.findall(pdf_pattern, html_content)

        # Remove duplicates while preserving order
        seen = set()
        unique_urls = []
        for url in pdf_urls:
            if url not in seen:
                seen.add(url)
                unique_urls.append(url)

        # Try to categorize PDFs based on surrounding text or patterns
        # For now, we'll just number them
        pdf_info = []
        for i, url in enumerate(unique_urls, 1):
            # Try to extract a meaningful name from the URL or context
            category = self._categorize_pdf(url, html_content)
            pdf_info.append({"category": category or f"PDF {i}", "url": url})

        print(f"✓ Found {len(pdf_info)} unique PDF URLs")
        return pdf_info

    def _categorize_pdf(self, pdf_url: str, html_content: str) -> str:
        """
        Try to categorize the PDF based on surrounding HTML context.

        Args:
            pdf_url: The PDF URL to categorize
            html_content: Full HTML content of the page

        Returns:
            Category name or None
        """
        # Look for text near the PDF link
        # The pattern is: section header text, then eventually a "See Records" button with the PDF link

        # Find the position of this PDF URL in the HTML
        url_pos = html_content.find(pdf_url)
        if url_pos == -1:
            return None

        # Look backwards in the HTML for section headers
        # Common patterns: "Open Men's Records", "Junior Women's Records", etc.
        search_text = html_content[max(0, url_pos - 1000) : url_pos]

        # Look for common record type patterns
        patterns = [
            r"(Open Men[\'s]* Records?)",
            r"(Junior Men[\'s]* Records?)",
            r"(Youth Men[\'s]* Records?)",
            r"(Masters Men[\'s]* Records?)",
            r"(Open Women[\'s]* Records?)",
            r"(Junior Women[\'s]* Records?)",
            r"(Youth Women[\'s]* Records?)",
            r"(Masters Women[\'s]* Records?)",
        ]

        for pattern in patterns:
            match = re.search(pattern, search_text, re.IGNORECASE)
            if match:
                return match.group(1).strip()

        return None

    def scrape_records(self) -> List[Dict[str, Any]]:
        """
        Download and parse every records PDF on the page.

        Returns:
            The records of all PDFs together: the WSO's whole set

        Raises if the page links no PDF, or if any PDF fails to download or
        parse, or parses to nothing; one missing PDF would otherwise delete
        its classes in the sync.
        """
        pdf_info = self.fetch_pdf_urls()
        urls = {info["url"] for info in pdf_info}
        if len(pdf_info) != self.EXPECTED_PDFS or len(urls) != len(pdf_info):
            raise ValueError(
                f"{self.wso_name}: found {len(urls)} distinct records PDFs ({len(pdf_info)} links) "
                f"on {self.records_page_url}, expected {self.EXPECTED_PDFS} (page changed?)"
            )

        print()
        print("PDFs to process:")
        for info in pdf_info:
            print(f"  • {info['category']}: {info['url']}")

        parts = []
        for i, info in enumerate(pdf_info, 1):
            print(f"\n{'=' * 80}")
            print(f"Processing {i}/{len(pdf_info)}: {info['category']}")
            print(f"{'=' * 80}\n")

            scraper = WSORecordsNewEnglandScraper(self.wso_name, info["url"])
            try:
                scraper.download_pdf()
                records = scraper.scrape_pdf()
            finally:
                scraper.cleanup()
            print(f"Found {len(records)} records")
            parts.append((f"{info['category']} ({info['url']})", records))

        return every_part(self.wso_name, parts)

    def send_summary_notification(self, result: Dict[str, int], record_count: int):
        """Send a summary Slack notification for the sync."""
        if result["inserted"] + result["updated"] + result["deleted"] == 0:
            return

        if not self.slack_webhook_url:
            print("⚠ Slack webhook not configured, skipping notification")
            return

        title = f"{self.wso_name} WSO Records Postgres - Automated Scrape Complete"
        message = (
            f"*{title}*\n\n"
            f"Processed *{record_count}* record rows\n"
            f"*{result['inserted']}* inserted, *{result['updated']}* updated, "
            f"*{result['deleted']}* deleted, *{result['unchanged']}* unchanged"
        )

        payload = {"text": message}

        try:
            response = requests.post(self.slack_webhook_url, json=payload, timeout=10)
            response.raise_for_status()
            print("✓ Slack summary notification sent")
        except Exception as e:
            print(f"⚠ Failed to send Slack notification: {e}")

    def run(self):
        """Main execution method."""
        print("=" * 80)
        print(f"NEW ENGLAND WSO - AUTOMATED SCRAPER {'(DRY RUN)' if self.dry_run else ''}")
        print("=" * 80)
        print()

        records = self.scrape_records()

        print(f"\n{'=' * 80}")
        print("FINAL SUMMARY")
        print(f"{'=' * 80}")
        print(f"Parsed {len(records)} records")

        if self.dry_run:
            for rec in records[:20]:
                print(
                    f"  {rec['age_category']:15} | {rec['gender']:6} | {rec['weight_class']:5} | "
                    f"Snatch: {str(rec.get('snatch_record') or '-'):4} | "
                    f"C&J: {str(rec.get('cj_record') or '-'):4} | "
                    f"Total: {str(rec.get('total_record') or '-'):4}"
                )
            if len(records) > 20:
                print(f"  ... and {len(records) - 20} more")

        result = sync_wso_records(
            self.wso_name, records, dry_run=self.dry_run, allow_shrink=self.allow_shrink
        )
        if result is not None:
            self.send_summary_notification(result, len(records))

        print("\n✅ Complete!")


def main():
    """Main entry point."""
    parser = argparse.ArgumentParser(
        description="Automated scraper for New England WSO Records (processes all PDFs)",
        epilog="Example: python scraper_newengland_auto.py --dry-run",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Parse and print without touching Postgres",
    )
    parser.add_argument(
        "--allow-shrink",
        action="store_true",
        help="Let the sync delete more than a quarter of the stored classes",
    )

    args = parser.parse_args()

    load_dotenv()

    scraper = NewEnglandAutoScraper(
        dry_run=args.dry_run, allow_shrink=args.allow_shrink
    )
    scraper.run()


if __name__ == "__main__":
    main()
