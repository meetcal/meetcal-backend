#!/usr/bin/env python3
"""
Mountain South WSO Records - Automated Multi-PDF Scraper

This script automatically scrapes the Mountain South WSO records page,
extracts the 2 PDF URLs (Men and Women) from the "MOUNTAIN SOUTH WSO RECORDS" section,
and processes them using the PDF scraper.

Every PDF is read before anything is written: Postgres gets the WSO's whole
set in one exact-set sync (``utils.sync_wso_records``), so a PDF that fails
to download or parses to nothing fails the run instead of deleting its
classes.

USAGE:
  Dry-run (test without making changes):
    source venv/bin/activate && python scraper_mountainsouth_auto.py --dry-run

  Live run (replace the WSO's Postgres records with every PDF's records):
    source venv/bin/activate && python scraper_mountainsouth_auto.py
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
from scraper_pdf_mountainsouth import WSORecordsMountainSouthScraper  # noqa: E402


class MountainSouthAutoScraper:
    """Automated scraper that fetches PDF URLs and processes them."""

    # The records PDFs the page links (Men and Women). The sync is an exact set, so
    # a run that finds a different number fails rather than syncing a WSO
    # with a PDF missing; after checking the page by hand, update this.
    EXPECTED_PDFS = 2

    def __init__(self, dry_run: bool = False, allow_shrink: bool = False):
        """
        Initialize auto scraper.

        Args:
            dry_run: If True, parse and print without touching Postgres
            allow_shrink: Let the sync delete more than a quarter of the stored classes
        """
        self.records_page_url = "https://mountainsouth.org/records/"
        self.wso_name = "Mountain South"
        self.dry_run = dry_run
        self.allow_shrink = allow_shrink
        self.slack_webhook_url = os.getenv("SLACK_WEBHOOK_URL")

    def fetch_pdf_urls(self) -> List[Dict[str, str]]:
        """
        Fetch the records page and extract the 2 PDF URLs from "MOUNTAIN SOUTH WSO RECORDS" section.

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

        # Strategy: Find "MOUNTAIN SOUTH WSO RECORDS" section, extract only the 2 PDFs
        # that start with "Mountain South WSO Records" (not the certificate link)

        # Find the section
        section_start = html_content.find("MOUNTAIN SOUTH WSO RECORDS")
        if section_start == -1:
            print("⚠ Could not find 'MOUNTAIN SOUTH WSO RECORDS' section")
            return []

        # Find where the next section starts (ARCHIVED records)
        archived_start = html_content.find(
            "ARCHIVED MOUNTAIN SOUTH WSO RECORDS", section_start
        )

        # Extract only the current records section
        if archived_start != -1:
            section_html = html_content[section_start:archived_start]
        else:
            # Take a reasonable chunk
            section_html = html_content[section_start : section_start + 5000]

        # Find all links in this section
        # Look for: "Mountain South WSO Records" followed by date and "MEN" or "WOMEN"
        pattern = r'href="(https://mountainsouth\.org/[^"]+/Mountain-South-WSO-Records[^"]*\.pdf)"[^>]*>([^<]+)'
        matches = re.findall(pattern, section_html, re.IGNORECASE)

        pdf_info = []
        seen_urls = set()

        for url, link_text in matches:
            if url in seen_urls:
                continue

            # Skip certificate links
            if "certificate" in url.lower() or "certificate" in link_text.lower():
                continue

            # Determine if it's Men or Women based on URL or link text
            if "MEN" in url.upper() and "WOMEN" not in url.upper():
                category = "Men"
            elif "WOMEN" in url.upper():
                category = "Women"
            elif "MEN" in link_text.upper() and "WOMEN" not in link_text.upper():
                category = "Men"
            elif "WOMEN" in link_text.upper():
                category = "Women"
            else:
                # Try to guess from context
                continue

            pdf_info.append({"category": category, "url": url})
            seen_urls.add(url)

            # Stop once we have both
            if len(pdf_info) >= 2:
                break

        # If we didn't find them with link text, try a simpler approach
        if len(pdf_info) < 2:
            pdf_info = []
            seen_urls = set()

            # Extract all PDF URLs from the section
            all_pdfs = re.findall(
                r'(https://mountainsouth\.org/[^"\'>\s]+Mountain-South-WSO-Records[^"\'>\s]+\.pdf)',
                section_html,
                re.IGNORECASE,
            )

            for url in all_pdfs:
                if url in seen_urls:
                    continue

                # Skip certificates
                if "certificate" in url.lower():
                    continue

                # Determine category from URL
                if "MEN" in url.upper() and "WOMEN" not in url.upper():
                    category = "Men"
                elif "WOMEN" in url.upper():
                    category = "Women"
                else:
                    # If unclear, add as generic
                    if len(pdf_info) == 0:
                        category = "Men"
                    else:
                        category = "Women"

                pdf_info.append({"category": category, "url": url})
                seen_urls.add(url)

                if len(pdf_info) >= 2:
                    break

        print(
            f"✓ Found {len(pdf_info)} PDF URLs from MOUNTAIN SOUTH WSO RECORDS section"
        )
        return pdf_info

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

            scraper = WSORecordsMountainSouthScraper(self.wso_name, info["url"])
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
        print(f"MOUNTAIN SOUTH WSO - AUTOMATED SCRAPER {'(DRY RUN)' if self.dry_run else ''}")
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
        description="Automated scraper for Mountain South WSO Records (processes Men and Women PDFs)",
        epilog="Example: python scraper_mountainsouth_auto.py --dry-run",
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

    scraper = MountainSouthAutoScraper(
        dry_run=args.dry_run, allow_shrink=args.allow_shrink
    )
    scraper.run()


if __name__ == "__main__":
    main()
