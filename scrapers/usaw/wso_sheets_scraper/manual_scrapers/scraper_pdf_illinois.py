#!/usr/bin/env python3
"""
PDF scraper for Illinois WSO records.

One line per lift. Since October 2026 a line reads "JR F 61 Snatch 42 kg
BAKER, Sophie Oct 3, 2026 2026 Mid American Championships" (record in kg,
holder or STANDARD, date, and the meet as the place, blank for older
records); the earlier PDFs read "U13 F 37 Snatch 10 STANDARD 2026-08-01".
Both are read.

The Postgres set is synced exactly, and the PDF decides which youth groups
exist (the October 2026 one added U11 and dropped U13 and U15), so an age
group gone from both genders is taken as the source's choice. What fails
instead, as a parse that lost part of the PDF: too few classes or lifts, an
adult group missing (Junior, Senior, Masters 35-90), a gap in the pages'
footers ("4 of 15"), or an age group with a different number of classes for
women than for men. Same checks as meetcal-app's
convex/scrapers/parse/wso/illinois.ts.
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


# Totals the PDF has wrong, each checked against the athlete's meet results:
# (age category, gender, weight class, total as written, total). One applies
# only while the PDF still says what it was checked against, so a corrected
# (or changed) PDF wins.
TOTAL_CORRECTIONS = (
    # Stephanie Rosario, 2026 Mid American Championships: 66 + 84 = 150.
    ("Masters 40", "Women", "69", 1580, 150),
)


class WSORecordsIllinoisScraper:
    # A holder's name can run into the date ("LLOP KASSINGER, CarmenOct 3,
    # 2026"), so months are whole words only ("MarkOct 4" isn't March), and
    # PyPDF2 runs a page's last line into the next page's title.
    ROW_PATTERN = re.compile(
        r"^(?P<age>U\d+|JR|Open|[WM]\d{2})\s+"
        r"(?P<gender>[FM])\s+"
        r"(?P<weight>(?:>\s*)?\d+\+?)\s+"
        r"(?P<lift>Snatch|Clean\s*&\s*Jerk|Total)\s+"
        r"(?P<record>\d+(?:\.\d+)?)(?:\s*kg)?\s+"
        r"(?P<holder>.+?)\s*"
        r"(?P<date>\d{4}-\d{2}-\d{2}"
        r"|(?:Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|June?|July?|Aug(?:ust)?"
        r"|Sep(?:t(?:ember)?)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)\.?\s+\d{1,2},?\s+\d{4})"
        r"(?!\d)",
        re.IGNORECASE,
    )
    RECORD_ROW_PREFIX = re.compile(
        r"^(?:U\d+|JR|Open|[WM]\d{2})\s+[FM]\s+", re.IGNORECASE
    )
    # The October 2026 PDF has 256 classes (768 lifts), 128 a gender; the
    # September one had 282. Low enough that Illinois can drop more youth
    # groups; a partial parse is caught by the checks in _validate_records,
    # not by this floor.
    MIN_RECORD_ROWS = 150
    MIN_LIFT_VALUES = 3 * MIN_RECORD_ROWS
    ADULT_AGE_GROUPS = ("Junior", "Senior", *(f"Masters {age}" for age in range(35, 91, 5)))
    # Each page's footer: "1 of 15 IL WSO Records 20261004.xlsx" since October
    # 2026, "Page 1 of 27" before. The notes pages at the end have none.
    PAGE_FOOTER = re.compile(r"^(?:Page\s+)?(\d{1,3})\s+of\s+\d{1,3}(?:\s|$)", re.IGNORECASE)
    STANDARD_HOLDER = re.compile(
        r"(?:(?:world|record|wso|state|american|national)\s+)?standard|", re.IGNORECASE
    )

    def __init__(self, wso_name: str, pdf_url: str):
        self.wso_name = wso_name
        self.pdf_url = pdf_url
        self.ingest_client: Optional[Any] = None
        self.slack_webhook_url: Optional[str] = None
        self.pdf_path = "temp_illinois_wso_records.pdf"
        self.parse_warnings: List[str] = []
        # Who holds each kept lift value, by (class key, field), for the total check.
        self._holders: Dict[Tuple[Tuple[str, str, str], str], str] = {}

    def setup_ingest_client(self):
        from common.postgres_ingest import IngestClient

        self.ingest_client = IngestClient()
        print("Postgres ingest client initialized")

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
        holder: str,
        source_line: str,
    ) -> None:
        key = (record["age_category"], record["gender"], record["weight_class"])
        if field not in record:
            record[field] = value
            self._holders[(key, field)] = holder
            return

        existing = record[field]
        if existing == value:
            return
        if existing == 0 and value > 0:
            record[field] = value
            self._holders[(key, field)] = holder
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

    def _validate_records(
        self, records: List[Dict[str, Any]], pages: List[int]
    ) -> None:
        # A page the extraction lost leaves a gap in the footers' page numbers.
        gaps = [page for page in range(1, max(pages, default=0) + 1) if page not in pages]
        if gaps:
            raise ValueError(
                f"Illinois PDF is missing pages {', '.join(map(str, gaps))} "
                "(by its page footers)"
            )
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

        for gender in ("Men", "Women"):
            actual = {
                record["age_category"]
                for record in records
                if record["gender"] == gender
            }
            missing = [age for age in self.ADULT_AGE_GROUPS if age not in actual]
            if missing:
                raise ValueError(
                    f"Illinois PDF is missing {gender} age groups: {', '.join(missing)}"
                )

        # Illinois lists as many classes for women as for men in every age
        # group (both PDFs so far), so a group short for one gender lost rows.
        classes: Dict[str, Dict[str, int]] = {}
        for record in records:
            counts = classes.setdefault(record["age_category"], {"Women": 0, "Men": 0})
            counts[record["gender"]] += 1
        for age, counts in classes.items():
            if counts["Women"] != counts["Men"]:
                raise ValueError(
                    f"Illinois PDF has {counts['Women']} Women and {counts['Men']} Men "
                    f"{age} classes (part of the PDF not read?)"
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
            self._check_total(record, identity)

    def _check_total(self, record: Dict[str, Any], identity: str) -> None:
        """A total above snatch + clean & jerk can't be right, but the three
        numbers don't say which one is wrong: William Lund's M50 >110 total of
        147 (October 2026) is his meet total, and the 84 kg clean & jerk
        beside it is the slip (he made 85). A rule that guessed would
        overwrite a right total whenever a lift was the typo, so such a total
        is kept as written and logged, and corrected only once checked
        (TOTAL_CORRECTIONS). Standards over their lifts' sum are not logged:
        the conversion rules set a standard total apart from its lifts (see
        the PDF's last page). Same rule as meetcal-app's
        convex/scrapers/parse/wso/illinois.ts.
        """
        key = (record["age_category"], record["gender"], record["weight_class"])
        total = record.get("total_record")
        for *correction_key, written, corrected in TOTAL_CORRECTIONS:
            if tuple(correction_key) == key and total == written:
                self.parse_warnings.append(
                    f"Corrected source total {written} to {corrected} "
                    f"(checked against results): {identity}"
                )
                record["total_record"] = corrected
                return
        snatch = record.get("snatch_record")
        cj = record.get("cj_record")
        if not snatch or not cj or not total or total <= snatch + cj:
            return
        holder = re.sub(r"\s+", " ", self._holders.get((key, "total_record"), "")).strip()
        if not self.STANDARD_HOLDER.fullmatch(holder):
            self.parse_warnings.append(
                f"Source total ({total}) is above snatch + clean & jerk ({snatch + cj}); "
                f"kept as written: {identity}"
            )

    def parse_pdf_text(
        self, text: str, *, validate: bool = True
    ) -> List[Dict[str, Any]]:
        self.parse_warnings = []
        self._holders = {}
        grouped: Dict[Tuple[str, str, str], Dict[str, Any]] = {}
        unparsed_record_lines: List[str] = []
        pages: List[int] = []
        lift_fields = {
            "snatch": "snatch_record",
            "clean&jerk": "cj_record",
            "total": "total_record",
        }

        for raw_line in text.splitlines():
            line = raw_line.strip()
            footer = self.PAGE_FOOTER.match(line)
            if footer:
                pages.append(int(footer.group(1)))
                continue
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
            self._set_lift_value(record, field, value, match.group("holder"), line)

        if unparsed_record_lines:
            examples = "\n".join(f"  {line}" for line in unparsed_record_lines[:5])
            raise ValueError(
                f"Could not parse {len(unparsed_record_lines)} Illinois record rows:\n"
                f"{examples}"
            )

        records = list(grouped.values())
        if validate:
            self._validate_records(records, pages)
        return records

    def scrape_pdf(self) -> List[Dict[str, Any]]:
        records = self.parse_pdf_text(self.extract_pdf_text())
        for warning in self.parse_warnings:
            print(f"Parser warning: {warning}")
        return records

    def replace_in_postgres(self, records: List[Dict[str, Any]]) -> Dict[str, int]:
        if not self.ingest_client:
            raise ValueError("Ingest client not initialized")

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
            "wso": self.wso_name,
            "records": payload_records,
        }
        return self.ingest_client.action("scraperIngestion:replaceWSORecordSet", payload)

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
                self.setup_ingest_client()
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

            result = self.replace_in_postgres(records)
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
    parser.add_argument("--dry-run", action="store_true", help="Parse without updating Postgres")
    args = parser.parse_args()

    load_dotenv()

    scraper = WSORecordsIllinoisScraper(args.wso, args.pdf_url)
    scraper.run(dry_run=args.dry_run)


if __name__ == "__main__":
    main()
