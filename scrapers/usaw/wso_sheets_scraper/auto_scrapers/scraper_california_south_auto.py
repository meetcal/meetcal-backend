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
from common.convex_compat import ConvexClient
from dotenv import load_dotenv

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from utils import wso_record_ingest_args

load_dotenv()


class WSORecordsCaliforniaSouthScraper:
    """Scraper for California South WSO weightlifting records."""

    SHEET_ID = "1PHYJ-lhkXYMrQIIo6YaipePFxruSfbRw1TEUtIoknR0"

    def __init__(self, wso_name: str, sheet_url: str):
        self.wso_name = wso_name
        self.sheet_url = sheet_url
        self.convex_client = None
        self.scraper_secret = None

    def setup_convex_client(self):
        self.convex_client = ConvexClient(os.getenv("CONVEX_URL"))
        self.scraper_secret = os.getenv("SCRAPER_SECRET")
        print("✓ Convex client initialized")

    def _normalize_age_group(self, age_group: str) -> str:
        """Map sheet ageGroup labels to MeetCal age_category values."""
        age = (age_group or "").strip()
        upper = age.upper()

        if upper.startswith("JR"):
            return "Junior"
        if upper.startswith("OPEN"):
            return "Senior"
        match = re.match(r"^[MW](\d+)(.*)$", age, re.IGNORECASE)
        if match:
            suffix = match.group(2) or ""
            return f"Masters {match.group(1)}{suffix}"
        return age

    def _parse_weight_class(self, weight_min: str, weight_max: str) -> Optional[str]:
        """
        Build a MeetCal weight_class from the body-weight range columns.

        The sheet uses an open lower bound (min=0) for the first real class, so
        when min is 0 or empty we treat max as the class (e.g. 0-30 -> "30").
        When min is set, the class is the lower bound (e.g. 30-33 -> "30").
        An empty max with a min means an open-ended class (e.g. "61" -> "61+").
        """
        weight_min = (weight_min or "").strip()
        weight_max = (weight_max or "").strip()
        if not weight_min and not weight_max:
            return None
        if not weight_max:
            return f"{weight_min}+"
        if weight_max.startswith(">"):
            return weight_max.lstrip(">") + "+"
        if not weight_min:
            return weight_max
        try:
            if int(float(weight_min)) <= 0:
                return weight_max
        except ValueError:
            pass
        return weight_min

    def scrape_sheet(self) -> List[Dict]:
        csv_url = (
            f"https://docs.google.com/spreadsheets/d/{self.SHEET_ID}"
            f"/gviz/tq?tqx=out:csv"
        )
        response = requests.get(csv_url, timeout=60)
        if response.status_code != 200:
            raise RuntimeError(f"Failed to fetch sheet: HTTP {response.status_code}")

        grouped: Dict[tuple, dict] = {}
        for row in csv.DictReader(io.StringIO(response.text)):
            age_raw = row.get("ageGroup", "").strip()
            gender_raw = row.get("gender", "").strip()
            lift_type = row.get("lift", "").strip()
            wso_record = row.get("WSO record", "").strip()

            if not age_raw or not gender_raw or not lift_type:
                continue

            age_category = self._normalize_age_group(age_raw)
            if "ADAP" in age_category:
                continue

            gender = "Women" if gender_raw == "F" else "Men" if gender_raw == "M" else None
            if not gender:
                continue

            weight_class = self._parse_weight_class(
                row.get("bodyWeightMin", ""),
                row.get("bodyWeightMax", ""),
            )
            if not weight_class:
                continue

            key = (age_category, gender, weight_class)
            entry = grouped.setdefault(
                key,
                {"snatch": None, "cj": None, "total": None},
            )

            lift_lower = lift_type.lower()
            try:
                value = int(float(wso_record)) if wso_record else None
            except ValueError:
                value = None

            if lift_lower == "snatch":
                entry["snatch"] = value
            elif lift_lower in ("clean & jerk", "clean and jerk", "c&j", "cleanjerk"):
                entry["cj"] = value
            elif lift_lower == "total":
                entry["total"] = value

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

    def upsert_records(self, records: List[Dict]) -> None:
        for record in records:
            self.convex_client.action(
                "scraperIngestion:ingestWSORecord",
                wso_record_ingest_args(record, self.scraper_secret),
            )
            print(
                f"  ✓ Upserted: {record['age_category']} "
                f"{record['gender']} {record['weight_class']}"
            )

    def run(self) -> None:
        print(f"Starting scraper for {self.wso_name}")
        print(f"Sheet URL: {self.sheet_url}")

        self.setup_convex_client()

        print("Scraping Google Sheet...")
        records = self.scrape_sheet()
        print(f"Found {len(records)} records")

        print("Upserting records to Convex...")
        self.upsert_records(records)

        print("Done!")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="WSO Records Scraper (California South Format)"
    )
    parser.add_argument("--wso", required=True, help="WSO name (should be 'California South')")
    parser.add_argument("--sheet-url", required=True, help="Google Sheet URL")
    args = parser.parse_args()

    scraper = WSORecordsCaliforniaSouthScraper(args.wso, args.sheet_url)
    scraper.run()


if __name__ == "__main__":
    main()
