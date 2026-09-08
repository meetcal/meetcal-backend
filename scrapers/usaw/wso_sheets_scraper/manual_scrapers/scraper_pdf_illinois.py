#!/usr/bin/env python3
"""
PDF scraper for Illinois WSO records.
"""

import argparse
import os
import re
import sys
from typing import Any, Dict, List, Optional, Tuple

import requests
from PyPDF2 import PdfReader
from dotenv import load_dotenv

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

class WSORecordsIllinoisScraper:
    ROW_PATTERN = re.compile(
        r"^(?P<age>U\d+|JR|Open|[WM]\d{2})\s+"
        r"(?P<gender>[FM])\s+"
        r"(?P<weight>(?:>\s*)?\d+\+?)\s+"
        r"(?P<lift>Snatch|Clean\s*&\s*Jerk|Total)\s+"
        r"(?P<record>\d+(?:\.\d+)?)\s+"
        r".+?\b\d{4}-\d{2}-\d{2}(?!\d)",
        re.IGNORECASE,
    )
    RECORD_ROW_PREFIX = re.compile(
        r"^(?:U\d+|JR|Open|[WM]\d{2})\s+[FM]\s+", re.IGNORECASE
    )
    MIN_RECORD_ROWS = 250
    MIN_LIFT_VALUES = 750

    def __init__(self, wso_name: str, pdf_url: str):
        self.wso_name = wso_name
        self.pdf_url = pdf_url
        self.convex_client: Optional[Any] = None
        self.slack_webhook_url: Optional[str] = None
        self.pdf_path = "temp_illinois_wso_records.pdf"
        self.parse_warnings: List[str] = []

    def setup_convex_client(self):
        from common.convex_compat import ConvexClient

        convex_url = os.getenv("CONVEX_URL")
        if not convex_url:
            raise ValueError("CONVEX_URL must be set")
        self.convex_client = ConvexClient(convex_url)
        print("Convex client initialized")

    def setup_slack(self):
        self.slack_webhook_url = os.getenv("SLACK_WEBHOOK_URL")
        if self.slack_webhook_url:
            print("Slack webhook configured")

    def download_pdf(self):
        print(f"Downloading PDF from {self.pdf_url}...")
        response = requests.get(
            self.pdf_url,
            headers={
                "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
            },
            timeout=30,
        )
        response.raise_for_status()
        with open(self.pdf_path, "wb") as file:
            file.write(response.content)
        print(f"PDF downloaded to {self.pdf_path}")

    def extract_pdf_text(self) -> str:
        reader = PdfReader(self.pdf_path)
        pages = []
        for page in reader.pages:
            text = page.extract_text() or ""
            pages.append(text)
        return "\n".join(pages)

    def _normalize_weight_class(self, raw_weight_class: str) -> str:
        normalized = raw_weight_class.strip().lower().replace("kg", "")
        normalized = normalized.replace(" ", "")
        if normalized.startswith(">"):
            return normalized[1:].rstrip("+") + "+"
        if normalized.endswith("+"):
            return normalized[:-1] + "+"
        return normalized

    def _map_age_category(self, raw_age: str, raw_gender: str) -> str:
        age = raw_age.strip().upper()
        gender = raw_gender.strip().upper()
        if age.startswith("U"):
            return age
        if age == "JR":
            return "Junior"
        if age == "OPEN":
            return "Senior"

        masters_match = re.fullmatch(r"([WM])(\d{2})", age)
        if not masters_match:
            raise ValueError(f"Unsupported Illinois age group: {raw_age}")

        expected_prefix = "W" if gender == "F" else "M"
        if masters_match.group(1) != expected_prefix:
            raise ValueError(
                f"Illinois age/gender mismatch: age={raw_age}, gender={raw_gender}"
            )
        return f"Masters {masters_match.group(2)}"

    def _parse_record_value(self, raw_value: str) -> Any:
        value = float(raw_value)
        return int(value) if value.is_integer() else value

    def _set_lift_value(
        self,
        record: Dict[str, Any],
        field: str,
        value: Any,
        source_line: str,
    ) -> None:
        if field not in record:
            record[field] = value
            return

        existing = record[field]
        if existing == value:
            return
        if existing == 0 and value > 0:
            record[field] = value
            self.parse_warnings.append(
                f"Preferred non-zero duplicate ({value}) over zero: {source_line}"
            )
            return
        if value == 0 and existing > 0:
            self.parse_warnings.append(
                f"Ignored zero duplicate in favor of {existing}: {source_line}"
            )
            return

        raise ValueError(
            f"Conflicting Illinois values for {field}: {existing} and {value}: "
            f"{source_line}"
        )

    def _validate_records(self, records: List[Dict[str, Any]]) -> None:
        if len(records) < self.MIN_RECORD_ROWS:
            raise ValueError(
                f"Illinois PDF yielded only {len(records)} record rows; "
                f"expected at least {self.MIN_RECORD_ROWS}"
            )

        lift_fields = ("snatch_record", "cj_record", "total_record")
        lift_value_count = sum(
            field in record for record in records for field in lift_fields
        )
        if lift_value_count < self.MIN_LIFT_VALUES:
            raise ValueError(
                f"Illinois PDF yielded only {lift_value_count} lift values; "
                f"expected at least {self.MIN_LIFT_VALUES}"
            )

        expected_age_categories = {
            "U13",
            "U15",
            "U17",
            "Junior",
            "Senior",
            *(f"Masters {age}" for age in range(35, 91, 5)),
        }
        for gender in ("Men", "Women"):
            actual = {
                record["age_category"]
                for record in records
                if record["gender"] == gender
            }
            missing = expected_age_categories - actual
            if missing:
                raise ValueError(
                    f"Illinois PDF is missing {gender} age groups: "
                    f"{', '.join(sorted(missing))}"
                )

        lift_labels = {
            "snatch_record": "snatch",
            "cj_record": "clean & jerk",
            "total_record": "total",
        }
        for record in records:
            missing_lifts = [
                label for field, label in lift_labels.items() if field not in record
            ]
            identity = (
                f"{record['age_category']} {record['gender']} "
                f"{record['weight_class']}"
            )
            if missing_lifts:
                self.parse_warnings.append(
                    f"Source row is missing {', '.join(missing_lifts)}: {identity}"
                )

            total = record.get("total_record")
            individual_lifts = [
                record.get("snatch_record"), record.get("cj_record")
            ]
            positive_lifts = [value for value in individual_lifts if value is not None]
            if total and positive_lifts and total < max(positive_lifts):
                self.parse_warnings.append(
                    f"Source total ({total}) is below an individual lift "
                    f"({max(positive_lifts)}): {identity}"
                )

    def parse_pdf_text(
        self, text: str, *, validate: bool = True
    ) -> List[Dict[str, Any]]:
        self.parse_warnings = []
        grouped: Dict[Tuple[str, str, str], Dict[str, Any]] = {}
        unparsed_record_lines: List[str] = []
        lift_fields = {
            "snatch": "snatch_record",
            "clean&jerk": "cj_record",
            "total": "total_record",
        }

        for raw_line in text.splitlines():
            line = raw_line.strip()
            match = self.ROW_PATTERN.match(line)
            if not match:
                if self.RECORD_ROW_PREFIX.match(line):
                    unparsed_record_lines.append(line)
                continue

            raw_gender = match.group("gender").upper()
            gender = "Women" if raw_gender == "F" else "Men"
            age_category = self._map_age_category(match.group("age"), raw_gender)
            weight_class = self._normalize_weight_class(match.group("weight"))
            key = (age_category, gender, weight_class)
            record = grouped.setdefault(
                key,
                {
                    "wso": self.wso_name,
                    "age_category": age_category,
                    "gender": gender,
                    "weight_class": weight_class,
                },
            )
            normalized_lift = re.sub(r"\s+", "", match.group("lift").lower())
            field = lift_fields[normalized_lift]
            value = self._parse_record_value(match.group("record"))
            self._set_lift_value(record, field, value, line)

        if unparsed_record_lines:
            examples = "\n".join(f"  {line}" for line in unparsed_record_lines[:5])
            raise ValueError(
                f"Could not parse {len(unparsed_record_lines)} Illinois record rows:\n"
                f"{examples}"
            )

        records = list(grouped.values())
        if validate:
            self._validate_records(records)
        return records

    def scrape_pdf(self) -> List[Dict[str, Any]]:
        records = self.parse_pdf_text(self.extract_pdf_text())
        for warning in self.parse_warnings:
            print(f"Parser warning: {warning}")
        return records

    def replace_in_convex(self, records: List[Dict[str, Any]]) -> Dict[str, int]:
        if not self.convex_client:
            raise ValueError("Convex client not initialized")

        scraper_secret = os.getenv("SCRAPER_SECRET")
        if not scraper_secret:
            raise ValueError("SCRAPER_SECRET must be set")

        payload_records = []
        for record in records:
            payload_record = {
                "ageCategory": record["age_category"],
                "gender": record["gender"],
                "weightClass": record["weight_class"],
            }
            if record.get("snatch_record") is not None:
                payload_record["snatchRecord"] = record["snatch_record"]
            if record.get("cj_record") is not None:
                payload_record["cjRecord"] = record["cj_record"]
            if record.get("total_record") is not None:
                payload_record["totalRecord"] = record["total_record"]
            payload_records.append(payload_record)

        payload = {
            "scraperSecret": scraper_secret,
            "wso": self.wso_name,
            "records": payload_records,
        }
        return self.convex_client.action("scraperIngestion:replaceWSORecordSet", payload)

    def send_slack_notification(self, result: Dict[str, int], record_count: int):
        if result["inserted"] + result["updated"] + result["deleted"] == 0:
            return

        if not self.slack_webhook_url:
            print("Slack webhook not configured, skipping notification")
            return

        title = f"{self.wso_name} WSO Records Postgres Update (PDF)"
        message = (
            f"*{title}*\n\n"
            f"Processed *{record_count}* current record rows\n"
            f"*{result['inserted']}* inserted, *{result['updated']}* updated, "
            f"*{result['deleted']}* deleted, *{result['unchanged']}* unchanged"
        )

        response = requests.post(
            self.slack_webhook_url,
            json={"text": message},
            timeout=10,
        )
        response.raise_for_status()
        print("Slack notification sent")

    def cleanup(self):
        if os.path.exists(self.pdf_path):
            os.remove(self.pdf_path)
            print(f"Cleaned up {self.pdf_path}")

    def run(self, dry_run: bool = False):
        try:
            print("=" * 80)
            print(f"ILLINOIS WSO PDF SCRAPER{' (DRY RUN)' if dry_run else ''}")
            print("=" * 80)
            print(f"PDF URL: {self.pdf_url}")
            print()

            if not dry_run:
                self.setup_convex_client()
                self.setup_slack()

            self.download_pdf()
            records = self.scrape_pdf()

            if not records:
                raise ValueError("No Illinois WSO records were parsed from the PDF")

            print(f"Parsed {len(records)} records")

            if dry_run:
                print("Sample records:")
                for record in records[:10]:
                    snatch = record.get("snatch_record")
                    cj = record.get("cj_record")
                    total = record.get("total_record")
                    print(
                        f"  {record['age_category']:10} | {record['gender']:5} | "
                        f"{record['weight_class']:5} | {snatch if snatch is not None else '-':>3} | "
                        f"{cj if cj is not None else '-':>3} | "
                        f"{total if total is not None else '-':>3}"
                    )
                if len(records) > 10:
                    print(f"  ... and {len(records) - 10} more")
                return

            result = self.replace_in_convex(records)
            print(
                f"Sync result: inserted={result['inserted']}, updated={result['updated']}, "
                f"deleted={result['deleted']}, unchanged={result['unchanged']}"
            )
            self.send_slack_notification(result, len(records))
        finally:
            self.cleanup()


def main():
    parser = argparse.ArgumentParser(description="PDF scraper for Illinois WSO records")
    parser.add_argument("--wso", required=True, help="WSO name")
    parser.add_argument("--pdf-url", required=True, help="PDF URL")
    parser.add_argument("--dry-run", action="store_true", help="Parse without updating Convex")
    args = parser.parse_args()

    load_dotenv()

    scraper = WSORecordsIllinoisScraper(args.wso, args.pdf_url)
    scraper.run(dry_run=args.dry_run)


if __name__ == "__main__":
    main()
