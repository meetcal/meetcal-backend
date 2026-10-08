#!/usr/bin/env python3
"""
WSO Records Scraper - California South Format

California South WSO records live in a single public Google Sheet whose CSV
export is a flat, wide table: one row per lift (Snatch / Clean & Jerk / Total)
with body-weight class columns and gender encoded as M/F.

Source page:
  https://www.californiasouthwso.org/records
  (embedded sheet id 1PHYJ-lhkXYMrQIIo6YaipePFxruSfbRw1TEUtIoknR0)

Only the columns we store in wso_records are kept:
  age_category, gender, weight_class, snatch_record, cj_record, total_record
"""

import argparse
import csv
import io
import os
import re
import sys
from typing import Dict, List, Optional

import requests
from dotenv import load_dotenv

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from utils import sync_wso_records

FETCH_TIMEOUT_SECONDS = 60
CJ_LIFTS = ("clean & jerk", "clean and jerk", "c&j", "cleanjerk")


class WSORecordsCaliforniaSouthScraper:
    """Scraper for California South WSO weightlifting records."""

    def __init__(self, wso_name: str, sheet_url: str):
        self.wso_name = wso_name
        self.sheet_url = sheet_url

    def _normalize_age_group(self, age_group: str) -> str:
        """Map sheet ageGroup labels to MeetCal age_category values, keeping a
        suffix (" ADAP") so adaptive groups are still recognised and skipped."""
        age = (age_group or "").strip()
        upper = age.upper()

        if upper.startswith("JR"):
            return age.replace("JR", "Junior", 1).replace("jr", "Junior", 1)
        match = re.match(r"^(open)(.*)$", age, re.IGNORECASE | re.DOTALL)
        if match:
            return f"Senior{match.group(2)}"
        match = re.match(r"^[MW](\d+)(.*)$", age, re.IGNORECASE | re.DOTALL)
        if match:
            return f"Masters {match.group(1)}{match.group(2)}"
        return age

    def _parse_weight_class(self, weight_min: str, weight_max: str) -> Optional[str]:
        """
        Build a MeetCal weight_class from the body-weight range columns.

        The class is the upper bound (0-30 -> "30", 30-33 -> "33"); an empty
        max means an open-ended class (61- -> "61+"). Naming a class by its
        lower bound filed every value one class too light and made 30-33
        collide with 0-30.
        """
        weight_min = (weight_min or "").strip()
        weight_max = (weight_max or "").strip()
        if not weight_max:
            return f"{weight_min}+" if weight_min else None
        if ">" in weight_max:
            return weight_max.replace(">", "") + "+"
        return weight_max

    def scrape_sheet(self) -> List[Dict]:
        """The sheet's first tab, one lift per row, the value in "WSO record"
        (the "American Record" column beside it is not ours)."""
        sheet_id = self.sheet_url.split("/d/")[1].split("/")[0]
        csv_url = f"https://docs.google.com/spreadsheets/d/{sheet_id}/gviz/tq?tqx=out:csv"
        response = requests.get(csv_url, timeout=FETCH_TIMEOUT_SECONDS)
        if response.status_code != 200:
            raise RuntimeError(f"Failed to fetch sheet: HTTP {response.status_code}")

        grouped: Dict[tuple, dict] = {}
        for row in csv.DictReader(io.StringIO(response.text)):
            age_raw = (row.get("ageGroup") or "").strip()
            gender_raw = (row.get("gender") or "").strip()
            lift_lower = (row.get("lift") or "").strip().lower()
            wso_record = (row.get("WSO record") or "").strip()

            if not age_raw or not gender_raw:
                continue

            gender = "Women" if gender_raw == "F" else "Men" if gender_raw == "M" else None
            if not gender:
                continue

            age_category = self._normalize_age_group(age_raw)
            if "ADAP" in age_category:
                continue

            weight_class = self._parse_weight_class(
                row.get("bodyWeightMin"),
                row.get("bodyWeightMax"),
            )
            if not weight_class:
                continue

            if lift_lower == "snatch":
                field = "snatch"
            elif lift_lower in CJ_LIFTS:
                field = "cj"
            elif lift_lower == "total":
                field = "total"
            else:
                # Any other lift never creates its class.
                continue

            try:
                value = int(float(wso_record)) if wso_record else None
            except ValueError:
                value = None

            entry = grouped.setdefault(
                (age_category, gender, weight_class),
                {"snatch": None, "cj": None, "total": None},
            )
            entry[field] = value

        records = []
        for (age_category, gender, weight_class), lifts in grouped.items():
            records.append(
                {
                    "wso": self.wso_name,
                    "age_category": age_category,
                    "gender": gender,
                    "weight_class": weight_class,
                    "snatch_record": lifts["snatch"],
                    "cj_record": lifts["cj"],
                    "total_record": lifts["total"],
                }
            )
        return records

    def run(self, dry_run: bool = False, allow_shrink: bool = False) -> None:
        print(f"Starting scraper for {self.wso_name}{' (DRY RUN)' if dry_run else ''}")
        print(f"Sheet URL: {self.sheet_url}")

        print("Scraping Google Sheet...")
        records = self.scrape_sheet()
        print(f"Found {len(records)} records")
        if dry_run:
            for record in records:
                print(
                    f"  {record['age_category']} | {record['gender']} | "
                    f"{record['weight_class']}: snatch={record['snatch_record']}, "
                    f"cj={record['cj_record']}, total={record['total_record']}"
                )

        # One exact-set write: classes the sheet no longer lists are deleted
        sync_wso_records(self.wso_name, records, dry_run=dry_run, allow_shrink=allow_shrink)

        print("Done!")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="WSO Records Scraper (California South Format)"
    )
    parser.add_argument("--wso", required=True, help="WSO name (should be 'California South')")
    parser.add_argument("--sheet-url", required=True, help="Google Sheet URL (its first tab is read)")
    parser.add_argument("--dry-run", action="store_true", help="Parse and print without updating Postgres")
    parser.add_argument(
        "--allow-shrink",
        action="store_true",
        help="Let the sync delete more than a quarter of the stored classes",
    )
    args = parser.parse_args()

    # Only a write needs the database settings.
    if not args.dry_run:
        load_dotenv()

    scraper = WSORecordsCaliforniaSouthScraper(args.wso, args.sheet_url)
    scraper.run(dry_run=args.dry_run, allow_shrink=args.allow_shrink)


if __name__ == "__main__":
    main()
