"""The Python and JS placeholder member-id rules agree on a shared fixture.

``placeholder_member_id`` (common/normalize.py) and ``placeholderMemberId``
(usaw/entry_scraper/placeholder_member_id.js) are two spellings of one rule:
the entry scraper mints the id in JS and the writer recognises it in Python.
Sharing code across the languages is impractical, so both are checked against
``fixtures/placeholder_member_ids.json``. The JS half needs ``node`` and is
skipped, with a message, when it is not on PATH.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import unittest
from pathlib import Path

from common.normalize import placeholder_member_id

TESTS_DIR = Path(__file__).resolve().parent
FIXTURE = TESTS_DIR / "fixtures" / "placeholder_member_ids.json"
NODE_SCRIPT = TESTS_DIR / "placeholder_parity.js"


def _cases() -> list[dict]:
    return json.loads(FIXTURE.read_text(encoding="utf-8"))["cases"]


class PlaceholderParityTests(unittest.TestCase):
    def test_fixture_covers_the_tricky_shapes(self) -> None:
        names = [case["name"] for case in _cases()]
        self.assertIn("", names)
        self.assertIn(None, names)
        self.assertTrue(any(isinstance(n, str) and "\t" in n for n in names), "tab")
        self.assertTrue(any(isinstance(n, str) and "  " in n for n in names), "multiple spaces")
        self.assertTrue(any(isinstance(n, str) and n != n.strip() for n in names), "edge space")
        self.assertTrue(any(isinstance(n, str) and not n.isascii() for n in names), "unicode")
        self.assertTrue(any(isinstance(n, str) and "'" in n for n in names), "punctuation")

    def test_python_rule_matches_the_fixture(self) -> None:
        for case in _cases():
            with self.subTest(name=case["name"]):
                self.assertEqual(placeholder_member_id(case["name"]), case["expected"])

    def test_js_rule_matches_the_fixture(self) -> None:
        node = shutil.which("node")
        if node is None:
            self.skipTest(
                "node is not on PATH; the JS half of the placeholder parity check "
                "(usaw/entry_scraper/placeholder_member_id.js) did not run"
            )
        result = subprocess.run(
            [node, str(NODE_SCRIPT), str(FIXTURE)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        actual = json.loads(result.stdout)
        cases = _cases()
        self.assertEqual(len(actual), len(cases))
        for case, got in zip(cases, actual):
            with self.subTest(name=case["name"]):
                self.assertEqual(got, case["expected"])


if __name__ == "__main__":
    unittest.main()
