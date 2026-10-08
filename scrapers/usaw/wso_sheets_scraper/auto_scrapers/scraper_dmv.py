#!/usr/bin/env python3
"""
WSO Records Scraper - DMV Format

For DMV WSO which uses a flat CSV format with different column names
(Age Group, Weight Class, Lift, Record, Gender with capitals and spaces).
"""

import os
import sys
import argparse
import re
from typing import List, Dict, Any, Optional
from collections import defaultdict
from urllib.parse import quote

import requests

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from utils import sync_wso_records

SHEET_NAME = "Current Records"
FETCH_TIMEOUT_SECONDS = 60


def gid_of(sheet_url: str) -> Optional[str]:
    """The tab (gid) a Google Sheets URL points at, if it names one."""
    match = re.search(r"[?&#]gid=(\d+)", sheet_url)
    return match.group(1) if match else None


def sheet_csv_url(sheet_url: str) -> str:
    """CSV of the tab the URL points at, else of the "Current Records" tab
    (meetcal-app's convex/scrapers/wsoRecords.ts flat() reads the same one)."""
    sheet_id = sheet_url.split('/d/')[1].split('/')[0]
    base = f"https://docs.google.com/spreadsheets/d/{sheet_id}/gviz/tq?tqx=out:csv"
    gid = gid_of(sheet_url)
    if gid:
        return f"{base}&gid={gid}"
    return f"{base}&sheet={quote(SHEET_NAME, safe='')}"


def print_records(records: List[Dict[str, Any]]) -> None:
    for record in records:
        print(
            f"  {record['age_category']} | {record['gender']} | {record['weight_class']}: "
            f"snatch={record['snatch_record']}, cj={record['cj_record']}, "
            f"total={record['total_record']}"
        )


class WSORecordsDMVScraper:
    """Scraper for DMV WSO weightlifting records with specific column format."""
    
    def __init__(self, wso_name: str, sheet_url: str):
        """Initialize the scraper with WSO name and sheet URL."""
        self.wso_name = wso_name
        self.sheet_url = sheet_url
        self.changes = {"inserted": [], "updated": []}
        self.slack_webhook_url = None
        
    def setup_slack(self):
        """Set up Slack webhook URL."""
        self.slack_webhook_url = os.getenv("SLACK_WEBHOOK_URL")
        if self.slack_webhook_url:
            print("✓ Slack webhook configured")
    
    def _normalize_age_group(self, age_group: str) -> str:
        """
        Normalize age group to match Ohio convention.
        
        Conversions:
        - JR, JR ADAP -> Junior, Junior ADAP
        - Open, Open ADAP, OPEN -> Senior, Senior ADAP
        - M35, W35 -> Masters 35 (M/W prefix removed, gender is separate)
        - M40 ADAP -> Masters 40 ADAP
        - U11, U13, U15, U17 -> keep as is
        """
        age_group = age_group.strip()
        age_group_upper = age_group.upper()
        
        # Handle JR -> Junior (case-insensitive)
        if age_group_upper.startswith('JR'):
            return age_group.replace('JR', 'Junior', 1).replace('jr', 'Junior', 1)
        
        # Handle Open/OPEN -> Senior (case-insensitive)
        if age_group_upper.startswith('OPEN'):
            # Preserve suffix (e.g., " ADAP") if present
            match = re.match(r'^(open)(.*)$', age_group, re.IGNORECASE)
            if match:
                suffix = match.group(2)
                return f"Senior{suffix}"
        
        # Handle M35, M40, W35, W40, etc. -> Masters 35, Masters 40
        # Pattern: M/W followed by digits
        match = re.match(r'^[MW](\d+)(.*)$', age_group, re.IGNORECASE)
        if match:
            age_num = match.group(1)
            suffix = match.group(2)  # e.g., " ADAP"
            return f"Masters {age_num}{suffix}"
        
        # Return as-is for U11, U13, U15, U17, etc.
        return age_group
    
    def scrape_sheet(self) -> List[Dict[str, Any]]:
        """
        Scrape data from Google Sheet in flat CSV format.
        
        Returns:
            List of records with structure:
            {
                'wso': str,
                'age_category': str,
                'gender': str,
                'weight_class': str,
                'snatch_record': int or None,
                'cj_record': int or None,
                'total_record': int or None
            }
        """
        response = requests.get(sheet_csv_url(self.sheet_url), timeout=FETCH_TIMEOUT_SECONDS)
        
        if response.status_code != 200:
            raise Exception(f"Failed to fetch sheet: {response.status_code}")
        
        # Parse CSV
        import csv
        import io
        csv_data = csv.DictReader(io.StringIO(response.text))
        
        # Group records by age_category + gender + weight_class
        grouped = defaultdict(lambda: {'snatch': None, 'cj': None, 'total': None})
        
        for row in csv_data:
            # Extract fields (DMV uses different column names with spaces and capitals)
            age_group_raw = row.get('Age Group', '').strip()
            gender_raw = row.get('Gender', '').strip()
            weight_min = row.get('bodyWeightMin', '').strip()
            weight_max = row.get('Weight Class', '').strip()
            lift_type = row.get('Lift', '').strip()
            record_value = row.get('Record', '').strip()
            
            # Skip empty rows
            if not age_group_raw or not gender_raw:
                continue
            
            # Convert gender: F -> Women, M -> Men
            gender = "Women" if gender_raw == "F" else "Men" if gender_raw == "M" else None
            if not gender:
                continue
            
            # Normalize age group to match Ohio convention
            age_group = self._normalize_age_group(age_group_raw)
            
            # Skip ADAP (adaptive) records
            if 'ADAP' in age_group:
                continue
            
            # Determine weight class:
            # If bodyWeightMax is empty but bodyWeightMin has a value, it means ">X" (e.g., >63)
            # Otherwise use bodyWeightMax
            if not weight_max and weight_min:
                # Empty max means "greater than min" -> use min with + suffix
                weight_class = weight_min + "+"
            elif weight_max:
                # Use max value, replace > with + suffix
                weight_class = weight_max.replace(">", "") + "+" if ">" in weight_max else weight_max
            else:
                # Both empty - skip this record
                continue
            
            # Parse record value
            record_int = None
            if record_value:
                try:
                    record_int = int(float(record_value))
                except ValueError:
                    pass
            
            # Create unique key
            key = (age_group, gender, weight_class)
            
            # Store the lift value (case-insensitive matching)
            lift_type_lower = lift_type.lower()
            if lift_type_lower == "snatch":
                grouped[key]['snatch'] = record_int
            elif lift_type_lower in ["clean & jerk", "clean and jerk", "c&j", "cleanjerk"]:
                grouped[key]['cj'] = record_int
            elif lift_type_lower == "total":
                grouped[key]['total'] = record_int
        
        # Convert grouped data to list of records
        records = []
        for (age_cat, gender, weight_class), lifts in grouped.items():
            records.append({
                'wso': self.wso_name,
                'age_category': age_cat,
                'gender': gender,
                'weight_class': weight_class,
                'snatch_record': lifts['snatch'],
                'cj_record': lifts['cj'],
                'total_record': lifts['total']
            })
        
        return records
    
    def send_slack_notification(self) -> None:
        """Send Slack notification with change summary."""
        if not self.slack_webhook_url:
            return
        total_inserted = len(self.changes["inserted"])
        total_updated = len(self.changes["updated"])

        if total_inserted == 0 and total_updated == 0:
            return
        else:
            message = f"*{self.wso_name} WSO Records Postgres Update*\n\n*Summary:*\n• {total_inserted} new record(s) inserted\n• {total_updated} record(s) updated"

            if total_inserted > 0:
                message += "\n\n🆕 *New Records*\n"
                for record in self.changes["inserted"][:10]:
                    lifts = []
                    if record.get("snatch_record"):
                        lifts.append(f"Snatch: {record['snatch_record']}kg")
                    if record.get("cj_record"):
                        lifts.append(f"C&J: {record['cj_record']}kg")
                    if record.get("total_record"):
                        lifts.append(f"Total: {record['total_record']}kg")

                    lifts_str = ", ".join(lifts) if lifts else "No records"
                    message += f"• *{record['age_category']}* | {record['gender']} | {record['weight_class']}\n  {lifts_str}\n"

                if total_inserted > 10:
                    message += f"_...and {total_inserted - 10} more_\n"

            if total_updated > 0:
                message += "\n📝 *Updated Records*\n"
                for record in self.changes["updated"][:10]:
                    changes_str = []
                    for field, change in record["changes"].items():
                        field_name = field.replace("_record", "").replace("cj", "C&J").title()
                        old = f"{change['old']}kg" if change['old'] else "None"
                        new = f"{change['new']}kg" if change['new'] else "None"
                        changes_str.append(f"{field_name}: {old} → {new}")

                    message += f"• *{record['age_category']}* | {record['gender']} | {record['weight_class']}\n  {', '.join(changes_str)}\n"

                if total_updated > 10:
                    message += f"_...and {total_updated - 10} more_\n"

            payload = {"text": message}

        try:
            response = requests.post(self.slack_webhook_url, json=payload)
            response.raise_for_status()
            print("✓ Slack notification sent")
        except Exception as e:
            print(f"✗ Failed to send Slack notification: {e}")
    
    def run(self, dry_run: bool = False, allow_shrink: bool = False) -> None:
        """Main execution flow."""
        print(f"Starting scraper for {self.wso_name}{' (DRY RUN)' if dry_run else ''}")
        print(f"Sheet URL: {self.sheet_url}")
        
        if not dry_run:
            self.setup_slack()
        
        # Scrape data
        print("Scraping Google Sheet...")
        records = self.scrape_sheet()
        print(f"Found {len(records)} records")
        if dry_run:
            print_records(records)
        
        # One exact-set write: classes the sheet no longer lists are deleted
        sync_wso_records(self.wso_name, records, dry_run=dry_run, allow_shrink=allow_shrink)
        
        if not dry_run:
            print("Sending Slack notification...")
            self.send_slack_notification()
        
        print("Done!")


def main():
    """Main entry point."""
    parser = argparse.ArgumentParser(description="WSO Records Scraper (DMV Format)")
    parser.add_argument("--wso", required=True, help="WSO name (should be 'DMV')")
    parser.add_argument("--sheet-url", required=True, help="Google Sheet URL (its gid picks the tab)")
    parser.add_argument("--dry-run", action="store_true", help="Parse and print without updating Postgres")
    parser.add_argument(
        "--allow-shrink",
        action="store_true",
        help="Let the sync delete more than a quarter of the stored classes",
    )
    
    args = parser.parse_args()
    
    scraper = WSORecordsDMVScraper(args.wso, args.sheet_url)
    scraper.run(dry_run=args.dry_run, allow_shrink=args.allow_shrink)


if __name__ == "__main__":
    main()
