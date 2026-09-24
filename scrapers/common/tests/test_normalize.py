"""Unit tests for the shared ingest normalisers. No database required."""

from __future__ import annotations

import unittest

from common.normalize import (
    KNOWN_PLATFORMS,
    is_placeholder_member_id,
    normalize_name,
    normalize_platform,
    normalize_time,
    parse_time,
    placeholder_member_id,
)


class NameAndMemberIdTests(unittest.TestCase):
    def test_normalize_name_matches_the_rust_rule(self):
        self.assertEqual(normalize_name("  Anna   McElderry  "), "anna mcelderry")
        self.assertEqual(normalize_name("ANNA MCELDERRY"), "anna mcelderry")
        self.assertEqual(normalize_name(None), "")

    def test_placeholder_member_id_is_deterministic_and_recognised(self):
        self.assertEqual(placeholder_member_id("Jane  Doe"), "noid:jane-doe")
        self.assertEqual(placeholder_member_id("jane doe"), placeholder_member_id("JANE DOE"))
        self.assertEqual(placeholder_member_id("O'Brien, Pat"), "noid:o-brien-pat")
        self.assertTrue(is_placeholder_member_id(""))
        self.assertTrue(is_placeholder_member_id("   "))
        self.assertTrue(is_placeholder_member_id(None))
        self.assertTrue(is_placeholder_member_id("noid:jane-doe"))
        self.assertFalse(is_placeholder_member_id("123456"))
        self.assertFalse(is_placeholder_member_id(123456))


class PlatformTests(unittest.TestCase):
    def test_known_platforms_are_canonicalised_case_insensitively(self):
        for platform in KNOWN_PLATFORMS:
            self.assertEqual(normalize_platform(platform.lower()), platform)
            self.assertEqual(normalize_platform(platform.upper()), platform)
            self.assertEqual(normalize_platform(f"  {platform}  "), platform)

    def test_unknown_platforms_are_title_cased_not_dropped(self):
        self.assertEqual(normalize_platform("gold"), "Gold")
        self.assertEqual(normalize_platform("PLATFORM  a"), "Platform A")
        # Same rule as the app's client-side canonicalisation: trim, collapse
        # whitespace, title-case each word.
        self.assertEqual(normalize_platform("  RED "), "Red")
        self.assertEqual(normalize_platform("\tgold\n platform  2 "), "Gold Platform 2")
        self.assertEqual(normalize_platform("   "), "")

    def test_non_strings_and_blanks_pass_through(self):
        self.assertIsNone(normalize_platform(None))
        self.assertEqual(normalize_platform("   "), "")
        self.assertEqual(normalize_platform(3), 3)


class TimeTests(unittest.TestCase):
    def test_accepted_shapes_map_to_h_mm_am_pm(self):
        cases = {
            "9:00": "9:00 AM",
            "09:00": "9:00 AM",
            "09:00:00": "9:00 AM",
            "14:30": "2:30 PM",
            "14:30:15": "2:30 PM",
            "0:30": "12:30 AM",
            "00:00": "12:00 AM",
            "12:00": "12:00 PM",
            "12:15 AM": "12:15 AM",
            "12:15 PM": "12:15 PM",
            "7:05 am": "7:05 AM",
            "7:05pm": "7:05 PM",
            "7:05 p.m.": "7:05 PM",
            "7:05:00 PM": "7:05 PM",
            "9 AM": "9:00 AM",
            " 9:00 AM ": "9:00 AM",
        }
        for raw, canonical in cases.items():
            with self.subTest(raw=raw):
                self.assertEqual(parse_time(raw), canonical)
                # Canonical form is a fixed point.
                self.assertEqual(parse_time(canonical), canonical)

    def test_unparseable_values_are_none_from_parse_and_kept_by_normalize(self):
        for raw in ["", "TBD", "9", "24:00", "13:00 PM", "9:60", "0:00 AM", "morning"]:
            with self.subTest(raw=raw):
                self.assertIsNone(parse_time(raw))
                self.assertEqual(normalize_time(raw), raw)
        self.assertIsNone(normalize_time(None))
        self.assertEqual(normalize_time(930), 930)


if __name__ == "__main__":
    unittest.main()
