#!/usr/bin/env python3

import os
import sys
import unittest


sys.path.insert(
    0,
    os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "manual_scrapers",
    ),
)

from scraper_pdf_illinois import WSORecordsIllinoisScraper


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


if __name__ == "__main__":
    unittest.main()
