#!/usr/bin/env python3

import os
import sys
import unittest
from collections import Counter


SCRAPER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "manual_scrapers"))
sys.path.insert(0, os.path.join(SCRAPER_DIR, "auto_scrapers"))

from scraper_pdf_illinois import WSORecordsIllinoisScraper
from scraper_illinois_auto import find_pdf_href

# IL-WSO-Records-20261004.pdf (revised October 4, 2026) as PyPDF2 extracts it.
OCTOBER_2026 = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "fixtures", "illinois_records_20261004.txt"
)


class IllinoisParserTests(unittest.TestCase):
    def setUp(self):
        self.scraper = WSORecordsIllinoisScraper(
            "Illinois", "https://example.com/illinois.pdf"
        )

    def parse(self, text: str):
        return self.scraper.parse_pdf_text(text, validate=False)

    def test_aggregates_lifts_and_normalizes_categories(self):
        records = self.parse(
            "\n".join(
                [
                    "U13 F >61 Snatch 46 STANDARD 2026-08-01",
                    "U13 F >61 Clean & Jerk 57 STANDARD 2026-08-01",
                    "U13 F >61 Total 109 STANDARD 2026-08-01",
                    "U15 F 57 Total 94 STANDARD 2026-08-01Women's USAW ILLINOIS WSO Records",
                    "JR M 110 Snatch 0 STANDARD 2026-08-01",
                    "Open M 110 Snatch 148 A Lifter 2026-08-01",
                    "W35 F 69 Total 144 A Lifter 2026-08-01",
                    "M40 M >110 Total 182 A Lifter 2026-08-01",
                ]
            )
        )

        self.assertEqual(
            records[0],
            {
                "wso": "Illinois",
                "age_category": "U13",
                "gender": "Women",
                "weight_class": "61+",
                "snatch_record": 46,
                "cj_record": 57,
                "total_record": 109,
            },
        )
        self.assertEqual(records[1]["age_category"], "U15")
        self.assertEqual(records[1]["total_record"], 94)
        self.assertEqual(records[2]["age_category"], "Junior")
        self.assertEqual(records[2]["snatch_record"], 0)
        self.assertEqual(records[3]["age_category"], "Senior")
        self.assertEqual(records[4]["age_category"], "Masters 35")
        self.assertEqual(records[5]["age_category"], "Masters 40")
        self.assertEqual(records[5]["weight_class"], "110+")

    def test_prefers_nonzero_value_over_duplicate_zero(self):
        records = self.parse(
            "\n".join(
                [
                    "U13 M 36 Total 0 STANDARD 2026-08-01",
                    "U13 M 36 Snatch 20 A Lifter 2026-08-01",
                    "U13 M 36 Clean & Jerk 28 A Lifter 2026-08-01",
                    "U13 M 36 Total 48 A Lifter 2026-08-01",
                ]
            )
        )

        self.assertEqual(records[0]["total_record"], 48)
        self.assertEqual(len(self.scraper.parse_warnings), 1)

    def test_rejects_conflicting_nonzero_values(self):
        with self.assertRaisesRegex(ValueError, "Conflicting Illinois values"):
            self.parse(
                "\n".join(
                    [
                        "U13 M 36 Total 47 A Lifter 2026-08-01",
                        "U13 M 36 Total 48 A Lifter 2026-08-01",
                    ]
                )
            )

    def test_rejects_unparsed_record_like_rows(self):
        with self.assertRaisesRegex(ValueError, "Could not parse 1 Illinois record rows"):
            self.parse("U13 F 30 Snatch INVALID STANDARD 2026-08-01")

    def test_full_validation_rejects_incomplete_documents(self):
        with self.assertRaisesRegex(ValueError, "yielded only 1 record rows"):
            self.scraper.parse_pdf_text(
                "U13 F 30 Snatch 0 STANDARD 2026-08-01"
            )


class IllinoisOctober2026Tests(unittest.TestCase):
    def setUp(self):
        self.scraper = WSORecordsIllinoisScraper(
            "Illinois", "https://example.com/illinois.pdf"
        )
        with open(OCTOBER_2026, encoding="utf-8") as file:
            self.records = self.scraper.parse_pdf_text(file.read())
        self.by_class = {
            (r["age_category"], r["gender"], r["weight_class"]): r for r in self.records
        }

    def test_reads_exactly_the_256_classes_u11_added_u13_u15_gone(self):
        self.assertEqual(len(self.records), 256)
        groups = Counter((r["gender"], r["age_category"]) for r in self.records)
        self.assertEqual(len(groups), 32)
        self.assertEqual(set(groups.values()), {8})
        self.assertEqual(groups[("Women", "U11")], 8)
        self.assertEqual(groups[("Men", "U11")], 8)
        for age in ("U13", "U15"):
            self.assertNotIn(("Women", age), groups)
            self.assertNotIn(("Men", age), groups)
        self.assertNotIn(("U17", "Women", "37"), self.by_class)
        self.assertNotIn(("U17", "Men", "32"), self.by_class)
        for record in self.records:
            for field in ("snatch_record", "cj_record", "total_record"):
                self.assertIn(field, record)

    def test_reads_kg_values_month_dates_places_and_a_name_run_into_its_date(self):
        self.assertEqual(
            self.by_class[("Junior", "Women", "61")],
            {
                "wso": "Illinois",
                "age_category": "Junior",
                "gender": "Women",
                "weight_class": "61",
                "snatch_record": 42,
                "cj_record": 54,
                "total_record": 96,
            },
        )
        # "LLOP KASSINGER, Carmen Oct 3, 2026 ..." (no space in the other extraction)
        self.assertEqual(self.by_class[("Masters 55", "Women", "61")]["total_record"], 94)
        self.assertEqual(self.by_class[("Junior", "Women", "86+")]["total_record"], 189)
        self.assertEqual(self.by_class[("U11", "Men", "65+")]["snatch_record"], 0)
        records = self.scraper.parse_pdf_text(
            "W55 F 61 Snatch 42 kg LLOP KASSINGER, CarmenOct 3, 2026 2026 Mid American CHampionships\n"
            "U11 F 41 Total 0 kg STANDARD Aug 1, 2026ILLINOIS UASW RECORDS",
            validate=False,
        )
        self.assertEqual(records[0]["snatch_record"], 42)
        self.assertEqual(records[1]["total_record"], 0)

    def test_corrects_only_a_checked_total_and_logs_any_other_excess(self):
        # Written 1580; she made 66 + 84 = 150 at the 2026 Mid American Championships.
        rosario = self.by_class[("Masters 40", "Women", "69")]
        self.assertEqual((rosario["snatch_record"], rosario["cj_record"], rosario["total_record"]), (66, 84, 150))
        # Kept: the PDF doesn't say whether the total or a lift is the slip
        # (Lund's lift is; Wegrzyn's total is).
        self.assertEqual(self.by_class[("Masters 50", "Men", "110+")]["total_record"], 147)
        self.assertEqual(self.by_class[("U17", "Women", "61")]["total_record"], 104)
        self.assertEqual(
            self.scraper.parse_warnings,
            [
                "Source total (104) is above snatch + clean & jerk (86); kept as written: U17 Women 61",
                "Corrected source total 1580 to 150 (checked against results): Masters 40 Women 69",
                "Source total (147) is above snatch + clean & jerk (146); kept as written: Masters 50 Men 110+",
            ],
        )

    def test_applies_a_correction_only_while_the_pdf_has_the_checked_value(self):
        for written, stored in (("150", 150), ("1590", 1590)):
            records = self.scraper.parse_pdf_text(
                self.lines_with("W40 F 69 Total 1580 kg", f"W40 F 69 Total {written} kg")
            )
            rosario = next(
                r for r in records
                if (r["age_category"], r["gender"], r["weight_class"]) == ("Masters 40", "Women", "69")
            )
            self.assertEqual(rosario["total_record"], stored)
        self.assertIn(
            "Source total (1590) is above snatch + clean & jerk (150); kept as written: Masters 40 Women 69",
            self.scraper.parse_warnings,
        )

    def lines_with(self, old: str, new: str) -> str:
        with open(OCTOBER_2026, encoding="utf-8") as file:
            text = file.read()
        self.assertIn(old, text)
        return text.replace(old, new)

    def test_keeps_a_right_total_when_a_lift_lost_a_digit(self):
        records = self.scraper.parse_pdf_text(
            self.lines_with("M50 M >110 Clean & Jerk 84 kg", "M50 M >110 Clean & Jerk 8 kg")
        )
        lund = next(
            r for r in records
            if (r["age_category"], r["gender"], r["weight_class"]) == ("Masters 50", "Men", "110+")
        )
        self.assertEqual((lund["cj_record"], lund["total_record"]), (8, 147))
        # 170 with a digit dropped is 62 + 8: still kept.
        text = (
            self.lines_with("JR F 77 Snatch 92 kg", "JR F 77 Snatch 62 kg")
            .replace("JR F 77 Clean & Jerk 115 kg", "JR F 77 Clean & Jerk 8 kg")
            .replace("JR F 77 Total 207 kg", "JR F 77 Total 170 kg")
        )
        junior = next(
            r for r in self.scraper.parse_pdf_text(text)
            if (r["age_category"], r["gender"], r["weight_class"]) == ("Junior", "Women", "77")
        )
        self.assertEqual(junior["total_record"], 170)

    def test_month_like_first_name_run_into_the_date(self):
        records = self.scraper.parse_pdf_text(
            self.lines_with("BINDER, Mark Oct 4, 2026", "BINDER, MarkOct 4, 2026")
        )
        self.assertEqual(len(records), 256)
        match = self.scraper.ROW_PATTERN.match(
            "M50 M 85 Snatch 79 kg BINDER, MarkOct 4, 2026 2026 Mid American Championships"
        )
        self.assertEqual((match.group("holder"), match.group("date")), ("BINDER, Mark", "Oct 4, 2026"))

    def test_rejects_a_missing_adult_age_group_as_a_lost_page_reads(self):
        with open(OCTOBER_2026, encoding="utf-8") as file:
            text = "\n".join(line for line in file if not line.startswith("W55 F "))
        with self.assertRaisesRegex(ValueError, "missing Women age groups: Masters 55"):
            self.scraper.parse_pdf_text(text)

    def without_page(self, n: int) -> str:
        """The fixture without page n: its lines up to and including its footer."""
        with open(OCTOBER_2026, encoding="utf-8") as file:
            lines = file.read().splitlines()
        footer = lambda page: next(
            i for i, line in enumerate(lines) if line.startswith(f"{page} of 15 ")
        )
        start = 0 if n == 1 else footer(n - 1) + 1
        return "\n".join(lines[:start] + lines[footer(n) + 1 :])

    def test_rejects_a_page_missing_by_its_footers(self):
        for page in (1, 4):
            with self.assertRaisesRegex(ValueError, f"missing pages {page} \\(by its page footers\\)"):
                self.scraper.parse_pdf_text(self.without_page(page))
        # The last records page (the end of Men U11, all of Men U17): its
        # footer is the last, so the classes don't match.
        with self.assertRaisesRegex(ValueError, "has 8 Women and 2 Men U11 classes"):
            self.scraper.parse_pdf_text(self.without_page(14))

    def test_rejects_a_group_read_for_one_gender_but_takes_one_gone_from_both(self):
        with open(OCTOBER_2026, encoding="utf-8") as file:
            lines = file.read().splitlines()
        with self.assertRaisesRegex(ValueError, "has 0 Women and 8 Men U11 classes"):
            self.scraper.parse_pdf_text(
                "\n".join(line for line in lines if not line.startswith("U11 F "))
            )
        # How the October PDF dropped U13 and U15.
        records = self.scraper.parse_pdf_text(
            "\n".join(line for line in lines if not line.startswith("U11 "))
        )
        self.assertEqual(len(records), 240)

    def test_rejects_one_genders_worth_of_classes(self):
        with open(OCTOBER_2026, encoding="utf-8") as file:
            women = "\n".join(line for line in file if " M " not in line[:8])
        with self.assertRaisesRegex(ValueError, "yielded only 128 record rows"):
            self.scraper.parse_pdf_text(women)


class IllinoisPdfLinkTests(unittest.TestCase):
    def page(self, href: str) -> str:
        # The page since October 2026: a banner at the top, the section ~60 KB further down.
        return (
            "<p>Illinois State Records are updated! Scroll down to see where you stand!</p>"
            + "<div>".ljust(60_000, ".")
            + '<h2>Illinois State Records</h2><a href="/s/guide.pdf">USAW Guide</a>'
            + f'<a href="{href}" class="sqs-block-button-element" target="_blank" > View the Records </a>'
        )

    def test_finds_the_link_past_the_banner(self):
        self.assertEqual(
            find_pdf_href(self.page("/s/IL-WSO-Records-20261004.pdf")),
            "/s/IL-WSO-Records-20261004.pdf",
        )
        self.assertEqual(find_pdf_href(self.page("/s/records-oct.pdf")), "/s/records-oct.pdf")

    def test_reads_from_the_heading_on_and_fails_without_one(self):
        html = (
            '<a href="/other.pdf">View Records</a> Illinois State Records <p>..</p>'
            '<a class="b" href="/s/IL-WSO-Records-20260913.pdf">View the Records</a>'
        )
        self.assertEqual(find_pdf_href(html), "/s/IL-WSO-Records-20260913.pdf")
        with self.assertRaisesRegex(ValueError, "Could not find"):
            find_pdf_href("<p>nothing</p>")

    def test_reads_the_records_sections_buttons_only_the_newest_by_date(self):
        # Squarespace's page: the banner and each block in its own <section>.
        def sections(*bodies: str) -> str:
            return "".join(f'<section class="page-section">{body}</section>' for body in bodies)

        def button(href: str) -> str:
            return f'<a href="{href}" class="sqs-block-button-element"> View the Records </a>'

        banner = '<p>Illinois State Records are updated!</p><a href="/s/club.pdf">View Records</a>'
        heading = (
            "<h2>Illinois State Records</h2>"
            "<h3>Records are updated to reflect the new IWF Categories!</h3>"
        )
        self.assertEqual(
            find_pdf_href(sections(banner, heading + button("/s/records-oct.pdf"))),
            "/s/records-oct.pdf",
        )
        self.assertEqual(
            find_pdf_href(
                sections(button("/s/IL-WSO-Records-20260913.pdf"), heading + button("/s/records-oct.pdf"))
            ),
            "/s/records-oct.pdf",
        )
        self.assertEqual(
            find_pdf_href(
                sections(
                    heading
                    + button("/s/IL-WSO-Records-20260913.pdf")
                    + button("/s/IL-WSO-Records-20261004.pdf")
                )
            ),
            "/s/IL-WSO-Records-20261004.pdf",
        )
        # Without sections: from the heading on, so a button above it is not taken.
        self.assertEqual(
            find_pdf_href(banner + heading + button("/s/records-oct.pdf")), "/s/records-oct.pdf"
        )
        with self.assertRaisesRegex(ValueError, "Could not find the Illinois records PDF URL"):
            find_pdf_href(sections(banner, heading))

    def test_finds_the_link_on_the_live_page_layout(self):
        # The Squarespace structure of illinoisweightlifting.com on 2026-10-07, trimmed.
        html = (
            '<section data-test="page-section"><p>Illinois State Records are updated!</p></section>'
            '<section data-test="page-section"><a href="https://assets.example/2025_USAW_Guide_to_Membership.pdf">'
            "View Records</a></section>"
            '<section data-test="page-section"><h2 style="text-align:center">Illinois State Records</h2>'
            "<h3>Records are updated to reflect the new IWF Categories!</h3>"
            '<a href="/s/IL-WSO-Records-20261004.pdf" class="sqs-block-button-element--medium" '
            'data-sqsp-button target="_blank" > View the Records </a></section>'
            "<section><h2>USAW Illinois Weightlifting WSO</h2></section>"
        )
        self.assertEqual(find_pdf_href(html), "/s/IL-WSO-Records-20261004.pdf")


if __name__ == "__main__":
    unittest.main()
