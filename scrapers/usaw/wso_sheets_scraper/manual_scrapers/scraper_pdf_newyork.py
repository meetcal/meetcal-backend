#!/usr/bin/env python3
"""
PDF Scraper for New York WSO Records

This scraper handles New York's table-structured PDF format.
Uses pdfplumber's table extraction for reliable parsing.

PDF Format:
- Proper table structure with columns: Wt. Class, Lift, Record, Name, Date, Event
- Each weight class has 3 rows: Snatch, Clean and Jerk, Total
- "Record Standard" in Name column indicates qualifying standard (still counts as record until beaten)
- Multiple sections for different age/gender categories (e.g., "Youth Men", "Youth Women")

Parse-only: one PDF holds only some of New York's classes, and Postgres is
synced with the WSO's whole set at once, so the writes go through
``auto_scrapers/scraper_newyork_auto.py``, which reads every PDF.

USAGE:
  Parse one PDF and print its records:
    source venv/bin/activate && python scraper_pdf_newyork.py --wso "New York" --pdf-url "https://www.nywso.com/_files/ugd/aba8a0_cb60cf1e1a9d4066ad8106c3e36526cc.pdf" --dry-run
"""

import os
import argparse
import tempfile
import requests
import pdfplumber
from typing import List, Dict, Any, Optional
from dotenv import load_dotenv


class WSORecordsNewYorkScraper:
    """Scraper for New York WSO records (table-structured PDF)."""
    
    def __init__(self, wso_name: str, pdf_url: str):
        """
        Initialize scraper.
        
        Args:
            wso_name: Name of the WSO (should be "New York")
            pdf_url: URL to the PDF file
        """
        self.wso_name = wso_name
        self.pdf_url = pdf_url
        self.pdf_path: Optional[str] = None

    def download_pdf(self):
        """Download PDF from URL."""
        print(f"Downloading PDF from {self.pdf_url}...")
        
        headers = {
            'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36'
        }
        
        response = requests.get(self.pdf_url, headers=headers, timeout=30)
        response.raise_for_status()
        
        # A temp file of its own, so PDFs read one after another never share a path.
        fd, self.pdf_path = tempfile.mkstemp(prefix="wso_records_", suffix=".pdf")
        with os.fdopen(fd, 'wb') as f:
            f.write(response.content)
        
        print(f"✓ PDF downloaded to {self.pdf_path}")
    
    def _normalize_weight_class(self, weight_str: str) -> Optional[str]:
        """Normalize weight class format."""
        if not weight_str:
            return None
        
        weight_str = str(weight_str).strip()
        
        # Handle 110+ or +110
        if "+" in weight_str:
            return weight_str.replace("+", "") + "+" if not weight_str.endswith("+") else weight_str
        
        return weight_str
    
    def _parse_int(self, value: str) -> Optional[int]:
        """Parse integer value, return None if invalid or 0."""
        if not value or value == "" or value == "0":
            return None
        
        try:
            parsed = int(float(str(value).strip()))
            return None if parsed == 0 else parsed
        except (ValueError, AttributeError):
            return None
    
    def _parse_section_header(self, header: str) -> tuple:
        """
        Parse section header to extract age category and gender.
        
        Examples (New York format):
        - "Youth Men" -> ("Youth", "Men")  # NY uses generic "Youth", not U13/U15/U17
        - "Youth Women" -> ("Youth", "Women")
        - "Junior Men" -> ("Junior", "Men")
        - "Open Men" -> ("Senior", "Men")
        - "Masters 35-39 Men" -> ("Masters 35", "Men")
        """
        header = header.strip()
        
        # Extract gender
        if "Men" in header:
            gender = "Men"
        elif "Women" in header:
            gender = "Women"
        else:
            return None, None
        
        # Extract age category
        if "Senior" in header or "Open" in header:
            return "Senior", gender
        elif "Junior" in header:
            return "Junior", gender
        elif "Youth" in header:
            # NY format: just "Youth Men" / "Youth Women" (no age subdivision)
            # Keep as "Youth" - they don't use U13/U15/U17
            return "Youth", gender
        elif "Masters" in header:
            # Extract age: "Masters 35-39" -> "Masters 35"
            import re
            match = re.search(r'(\d+)\s*-\s*\d+', header)
            if match:
                return f"Masters {match.group(1)}", gender
        
        return None, None
    
    def scrape_pdf(self) -> List[Dict[str, Any]]:
        """
        Scrape records from PDF using table extraction.
        
        Returns:
            List of record dictionaries
        """
        records = []
        
        with pdfplumber.open(self.pdf_path) as pdf:
            for page_num, page in enumerate(pdf.pages, 1):
                print(f"  Processing page {page_num}...")
                records.extend(self.parse_tables(page.extract_tables()))
        
        return records

    def parse_tables(self, tables: List[List[List[Optional[str]]]]) -> List[Dict[str, Any]]:
        """Records of one page's tables, as pdfplumber extracts them (rows of cells)."""
        records = []
        
        for table in tables:
            current_age_category = None
            current_gender = None
            current_weight_class = None
            current_snatch = None
            current_cj = None
            current_total = None
            
            for row in table:
                if not row or len(row) < 2:
                    continue
                
                # Check if this is a section header row
                # NY format: "Youth Men", "Youth Women", "Junior Men", "Senior Men", etc.
                first_cell = str(row[0] or "").strip()
                if ("Youth" in first_cell or "Junior" in first_cell or "Senior" in first_cell or "Open" in first_cell or "Masters" in first_cell) and \
                   ("Men" in first_cell or "Women" in first_cell):
                    age_cat, gender = self._parse_section_header(first_cell)
                    if age_cat and gender:
                        current_age_category = age_cat
                        current_gender = gender
                    continue
                
                # Skip header row
                if first_cell == "Class" or first_cell == "Lift":
                    continue
                
                # Check if this row starts a new weight class
                if first_cell and first_cell.replace("+", "").replace(" ", "").isdigit():
                    # Save previous weight class if complete
                    if current_weight_class and current_age_category and current_gender:
                        record = {
                            'wso': self.wso_name,
                            'age_category': current_age_category,
                            'gender': current_gender,
                            'weight_class': current_weight_class,
                            'snatch_record': current_snatch,
                            'cj_record': current_cj,
                            'total_record': current_total
                        }
                        records.append(record)
                    
                    # Start new weight class
                    current_weight_class = self._normalize_weight_class(first_cell)
                    current_snatch = None
                    current_cj = None
                    current_total = None
                
                # Parse lift data
                # Columns: Wt. Class, Lift, Record, Name, Date, Event
                if len(row) >= 4:
                    lift_type = str(row[1] or "").strip()
                    record_value = str(row[2] or "").strip()  # Column 2 has the weight value
                    
                    # "Record Standard" means qualifying standard (still counts as record)
                    # Remove " kg" from record value and parse
                    weight_value = record_value.replace(" kg", "").replace("kg", "").strip()
                    
                    # Empty weight means no record (treat as NULL)
                    if not weight_value:
                        weight_value = None
                    else:
                        weight_value = self._parse_int(weight_value)
                    
                    # Assign to appropriate lift type
                    if "Snatch" in lift_type:
                        current_snatch = weight_value
                    elif "C&J" in lift_type or "Clean" in lift_type:
                        current_cj = weight_value
                    elif "Total" in lift_type:
                        current_total = weight_value
            
            # Save last weight class
            if current_weight_class and current_age_category and current_gender:
                record = {
                    'wso': self.wso_name,
                    'age_category': current_age_category,
                    'gender': current_gender,
                    'weight_class': current_weight_class,
                    'snatch_record': current_snatch,
                    'cj_record': current_cj,
                    'total_record': current_total
                }
                records.append(record)
        
        return records
    
    def cleanup(self):
        """Remove temporary PDF file."""
        if self.pdf_path and os.path.exists(self.pdf_path):
            os.remove(self.pdf_path)
            print(f"✓ Cleaned up {self.pdf_path}")
        self.pdf_path = None
    
    def run(self):
        """Parse the PDF and print its records."""
        try:
            print(f"{'='*80}")
            print(f"WSO PDF SCRAPER - {self.wso_name}")
            print(f"{'='*80}")
            print(f"PDF URL: {self.pdf_url}\n")
            
            self.download_pdf()
            
            print("\nScraping PDF...")
            records = self.scrape_pdf()
            print(f"Found {len(records)} total records")
            
            for rec in records[:20]:
                print(f"  {rec['age_category']:15} | {rec['gender']:6} | {rec['weight_class']:5} | "
                      f"Snatch: {str(rec.get('snatch_record') or '-'):4} | "
                      f"C&J: {str(rec.get('cj_record') or '-'):4} | "
                      f"Total: {str(rec.get('total_record') or '-'):4}")
            if len(records) > 20:
                print(f"  ... and {len(records) - 20} more")
            
        finally:
            self.cleanup()


def main():
    """Main entry point."""
    parser = argparse.ArgumentParser(
        description="PDF Scraper for New York WSO Records (parse and print only)",
        epilog="Example: python scraper_pdf_newyork.py --wso 'New York' --pdf-url 'https://example.com/records.pdf' --dry-run"
    )
    parser.add_argument("--wso", required=True, help="WSO name (should be 'New York')")
    parser.add_argument("--pdf-url", required=True, help="URL to the PDF file")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Accepted for compatibility: this scraper never writes; scraper_newyork_auto.py syncs every PDF together",
    )
    
    args = parser.parse_args()
    
    load_dotenv()
    
    scraper = WSORecordsNewYorkScraper(args.wso, args.pdf_url)
    scraper.run()


if __name__ == "__main__":
    main()
