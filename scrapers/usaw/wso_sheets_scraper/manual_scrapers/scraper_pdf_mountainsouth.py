#!/usr/bin/env python3
"""
PDF Scraper for Mountain South WSO Records

This scraper handles Mountain South's table-structured PDF format.
Uses pdfplumber's table extraction for reliable parsing.

PDF Format:
- Table structure with columns: CAT, ATHLETE (First/Last), STATE, KG, DATE, EVENT, LOCATION
- Each weight class has 3 sections: Snatch, Clean & Jerk, Total
- Age/gender categories in section headers (e.g., "OPEN MEN", "JUNIOR MEN U20", "MASTERS MEN 35-39")
- Separate PDFs for Men and Women

Parse-only: one PDF holds only some of Mountain South's classes, and Postgres is
synced with the WSO's whole set at once, so the writes go through
``auto_scrapers/scraper_mountainsouth_auto.py``, which reads every PDF.

USAGE:
  Parse one PDF and print its records:
    source venv/bin/activate && python scraper_pdf_mountainsouth.py --wso "Mountain South" --pdf-url "https://mountainsouth.org/wp-content/uploads/2025/10/Mountain-South-WSO-Records-2025-10-19-MEN.pdf" --dry-run
"""

import os
import argparse
import tempfile
import requests
import pdfplumber
from typing import List, Dict, Any, Optional
from dotenv import load_dotenv


class WSORecordsMountainSouthScraper:
    """Scraper for Mountain South WSO records (table-structured PDF)."""
    
    def __init__(self, wso_name: str, pdf_url: str):
        """
        Initialize scraper.
        
        Args:
            wso_name: Name of the WSO (should be "Mountain South")
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
        
        Examples:
        - "OPEN MEN - SNATCH" -> ("Senior", "Men")
        - "JUNIOR MEN U20 - SNATCH" -> ("Junior", "Men")
        - "YOUTH MEN U17 - SNATCH" -> ("U17", "Men")
        - "MASTERS MEN 35-39 - SNATCH" -> ("Masters 35", "Men")
        """
        header = header.strip().upper()
        
        # Extract gender
        if "MEN" in header and "WOMEN" not in header:
            gender = "Men"
        elif "WOMEN" in header:
            gender = "Women"
        else:
            return None, None
        
        # Extract age category
        if "OPEN" in header:
            return "Senior", gender
        elif "JUNIOR" in header:
            return "Junior", gender
        elif "YOUTH" in header or "U17" in header or "U15" in header or "U13" in header:
            # Try to extract specific youth category
            if "U17" in header or "17" in header:
                return "U17", gender
            elif "U15" in header or "15" in header:
                return "U15", gender
            elif "U13" in header or "13" in header:
                return "U13", gender
            return "Youth", gender
        elif "MASTERS" in header:
            # Extract age: "MASTERS MEN 35-39" -> "Masters 35"
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
        with pdfplumber.open(self.pdf_path) as pdf:
            pages = []
            for page_num, page in enumerate(pdf.pages, 1):
                print(f"  Processing page {page_num}...")
                pages.append(page.extract_text() or "")
        
        return self.parse_pages(pages)

    def parse_pages(self, pages: List[str]) -> List[Dict[str, Any]]:
        """Records of the PDF's pages, as pdfplumber extracts their text."""
        records = []
        current_records = {}  # Key: (age_cat, gender, weight_class), Value: {snatch, cj, total}
        
        for text in pages:
            if not text:
                continue
            
            lines = text.split('\n')
            
            current_age_category = None
            current_gender = None
            current_lift_type = None  # "SNATCH", "CLEAN & JERK", or "TOTAL"
            
            for line in lines:
                line = line.strip()
                
                # Check for section headers (e.g., "OPEN MEN - SNATCH")
                if " - SNATCH" in line or " - CLEAN & JERK" in line or " - TOTAL" in line:
                    age_cat, gender = self._parse_section_header(line)
                    if age_cat and gender:
                        current_age_category = age_cat
                        current_gender = gender
                        
                        if "SNATCH" in line:
                            current_lift_type = "SNATCH"
                        elif "CLEAN" in line:
                            current_lift_type = "CLEAN_JERK"
                        elif "TOTAL" in line:
                            current_lift_type = "TOTAL"
                    continue
                
                # Skip header lines
                if "CAT" in line and "ATHLETE" in line:
                    continue
                if "Beginning 6/1/2025" in line:
                    continue
                
                # Parse data lines (weight class followed by optional record data)
                # Format: "60 FirstName LastName STATE KG DATE EVENT LOCATION"
                # or just: "60" (empty record)
                parts = line.split()
                if len(parts) > 0 and current_age_category and current_gender and current_lift_type:
                    # First part should be weight class
                    weight_class_str = parts[0]
                    if weight_class_str.replace("+", "").isdigit():
                        weight_class = self._normalize_weight_class(weight_class_str)
                        
                        # Initialize record if not exists
                        key = (current_age_category, current_gender, weight_class)
                        if key not in current_records:
                            current_records[key] = {
                                'snatch': None,
                                'cj': None,
                                'total': None
                            }
                        
                        # Try to extract KG value (4th element typically)
                        kg_value = None
                        if len(parts) >= 4:
                            # Try to find the KG value (should be a number)
                            for i in range(1, min(len(parts), 6)):
                                try:
                                    kg_value = self._parse_int(parts[i])
                                    if kg_value:
                                        break
                                except:
                                    continue
                        
                        # Store value in appropriate lift type
                        if current_lift_type == "SNATCH":
                            current_records[key]['snatch'] = kg_value
                        elif current_lift_type == "CLEAN_JERK":
                            current_records[key]['cj'] = kg_value
                        elif current_lift_type == "TOTAL":
                            current_records[key]['total'] = kg_value
        
        # Convert to list of records
        for (age_cat, gender, weight_class), values in current_records.items():
            record = {
                'wso': self.wso_name,
                'age_category': age_cat,
                'gender': gender,
                'weight_class': weight_class,
                'snatch_record': values['snatch'],
                'cj_record': values['cj'],
                'total_record': values['total']
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
        description="PDF Scraper for Mountain South WSO Records (parse and print only)",
        epilog="Example: python scraper_pdf_mountainsouth.py --wso 'Mountain South' --pdf-url 'https://example.com/records.pdf' --dry-run"
    )
    parser.add_argument("--wso", required=True, help="WSO name (should be 'Mountain South')")
    parser.add_argument("--pdf-url", required=True, help="URL to the PDF file")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Accepted for compatibility: this scraper never writes; scraper_mountainsouth_auto.py syncs every PDF together",
    )
    
    args = parser.parse_args()
    
    load_dotenv()
    
    scraper = WSORecordsMountainSouthScraper(args.wso, args.pdf_url)
    scraper.run()


if __name__ == "__main__":
    main()
