"""Unit tests for the meet automation pipeline's pure logic.

Run from the repo's scrapers dir:

    PYTHONPATH=. python -m unittest usaw.meet_automation.tests.test_meet_automation

These cover validation, PDF discovery, the Slack review blocks, and the
button-decision file handshake. They do not touch a database or the network.
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from usaw.meet_automation import config, detect, pipeline, slack
from usaw.meet_automation.config import SlackConfig
from usaw.meet_automation.models import SlackRef, SourceRefs, StagedBundle
from usaw.meet_automation.validate import validate

MEET = "2026 Test Championships"


def _athlete(**over):
    base = {
        "memberId": "1",
        "name": "Jane Doe",
        "age": 24,
        "club": "Test Barbell",
        "wso": "Ohio",
        "gender": "Female",
        "weightClass": "71",
        "entryTotal": 210,
        "sessionNumber": 1,
        "sessionPlatform": "Red",
        "meet": MEET,
        "adaptive": False,
    }
    base.update(over)
    return base


def _session(**over):
    base = {
        "date": "2026-06-20",
        "meet": MEET,
        "platform": "Red",
        "sessionId": 1,
        "startTime": "09:00:00",
        "weighInTime": "07:00:00",
        "weightClass": "71",
    }
    base.update(over)
    return base


class ValidateTests(unittest.TestCase):
    def test_clean_data_passes(self):
        report = validate([_athlete()], [_session()], MEET)
        self.assertTrue(report["ok"])
        self.assertEqual(report["errors"], 0)
        self.assertEqual(report["counts"]["athletes"], 1)

    def test_flags_bad_rows(self):
        bad = _athlete(
            name="Bad1",
            gender="X",
            weightClass="9",
            club="0 WSO RED",
            sessionNumber=None,
            sessionPlatform="",
        )
        report = validate([bad], [_session()], MEET)
        codes = {f["code"] for f in report["findings"]}
        self.assertFalse(report["ok"])
        self.assertIn("gender_invalid", codes)
        self.assertIn("weight_class_invalid", codes)
        self.assertIn("session_unassigned", codes)

    def test_platform_word_in_club_is_not_flagged(self):
        # Title-cased platform words are legitimate club names, not leaks.
        report = validate([_athlete(club="White Rose Barbell Club")], [_session()], MEET)
        codes = {f["code"] for f in report["findings"]}
        self.assertNotIn("club_competition_leak", codes)

    def test_allcaps_overlay_token_in_club_is_flagged(self):
        report = validate([_athlete(club="WSO JR Barbell")], [_session()], MEET)
        codes = {f["code"] for f in report["findings"]}
        self.assertIn("club_competition_leak", codes)

    def test_session_coverage_cross_check(self):
        # Athlete in session 2 but schedule only has session 1.
        report = validate([_athlete(sessionNumber=2)], [_session(sessionId=1)], MEET)
        codes = {f["code"] for f in report["findings"]}
        self.assertIn("athlete_session_not_in_schedule", codes)
        self.assertIn("schedule_session_no_athletes", codes)


class DiscoverPdfsTests(unittest.TestCase):
    def test_classifies_and_resolves_relative(self):
        html = (
            '<a href="https://cdn/x_START_LIST.pdf">Start List</a>'
            '<a href="/files/2026_Schedule.pdf">Schedule</a>'
        )
        start, sched, candidates = detect.discover_pdfs(
            html, base_url="https://www.usaweightlifting.org/2026-national-championships"
        )
        self.assertEqual(start, "https://cdn/x_START_LIST.pdf")
        self.assertEqual(
            sched, "https://www.usaweightlifting.org/files/2026_Schedule.pdf"
        )
        self.assertEqual(len(candidates), 2)

    def test_no_pdfs(self):
        start, sched, candidates = detect.discover_pdfs("<p>nothing here</p>")
        self.assertIsNone(start)
        self.assertIsNone(sched)
        self.assertEqual(candidates, [])


class SlackBlocksTests(unittest.TestCase):
    def _bundle(self):
        report = validate([_athlete()], [_session()], MEET)
        return StagedBundle(
            run_id="2026-test-run",
            watch_key="2026-test",
            meet_name=MEET,
            source=SourceRefs(page_url="https://x", start_list_url="https://x/s.pdf"),
            athletes=[_athlete()],
            schedule=[_session()],
            validation=report,
            slack=SlackRef(channel="C1", ts="1.2"),
        )

    def test_blocks_include_approve_reject_buttons(self):
        blocks = slack.build_blocks(SlackConfig(), self._bundle())
        actions = [b for b in blocks if b.get("type") == "actions"]
        self.assertEqual(len(actions), 1)
        action_ids = {e["action_id"] for e in actions[0]["elements"]}
        self.assertEqual(action_ids, {"meet_approve", "meet_reject"})
        # Each button carries the run id so the interactions endpoint knows it.
        for el in actions[0]["elements"]:
            self.assertEqual(el["value"], "2026-test-run")


class DecisionFileTests(unittest.TestCase):
    def test_read_and_consume_decision(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            original = config.STATE_DIR
            config.STATE_DIR = tmp_path  # redirect to temp
            try:
                run_id = "2026-test-run"
                decisions = tmp_path / "decisions"
                decisions.mkdir()
                path = decisions / f"{run_id}.json"
                path.write_text(json.dumps({"decision": "approved", "user_id": "U1"}))

                decision, dpath = pipeline._read_decision(run_id)
                self.assertEqual(decision, "approved")
                self.assertEqual(dpath, path)

                pipeline._consume_decision(dpath)
                self.assertFalse(path.exists())

                # Missing decision returns (None, None).
                self.assertEqual(pipeline._read_decision("nope"), (None, None))
            finally:
                config.STATE_DIR = original


class CliTests(unittest.TestCase):
    def test_approve_accepts_all_pending(self):
        # Regression: the documented cron flag must parse without erroring.
        args = pipeline.build_parser().parse_args(["approve", "--all-pending"])
        self.assertTrue(args.all_pending)
        self.assertIsNone(args.run_id)


class WatchesPathTests(unittest.TestCase):
    def test_default_load_honors_configured_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            shared = Path(tmp) / "shared_watches.json"
            shared.write_text(
                json.dumps([{"key": "k", "meet_name": "M", "page_url": "https://e.com"}])
            )
            original = config.WATCHES_PATH
            config.WATCHES_PATH = shared
            try:
                # load_watches() with no explicit path must read the shared file.
                watches = config.load_watches()
                self.assertEqual([w.key for w in watches], ["k"])
            finally:
                config.WATCHES_PATH = original


class _FakeResp:
    def __init__(self, data):
        self._data = data

    def json(self):
        return self._data

    def raise_for_status(self):
        pass


class _FakeRequests:
    def __init__(self, data):
        self._data = data

    def get(self, *a, **k):
        return _FakeResp(self._data)

    def post(self, *a, **k):
        return _FakeResp({"ok": True})


class ReplyAllowlistTests(unittest.TestCase):
    def _poll(self, allowed_users, reply_user):
        bundle = StagedBundle(
            run_id="r", watch_key="w", meet_name=MEET, slack=SlackRef(channel="C1", ts="root")
        )
        cfg = SlackConfig(bot_token="x", allowed_users=allowed_users)
        data = {
            "ok": True,
            "messages": [
                {"ts": "root"},  # the review message itself
                {"user": reply_user, "text": "okay"},
            ],
        }
        with mock.patch.dict(sys.modules, {"requests": _FakeRequests(data)}):
            return slack.poll_approval(cfg, bundle)

    def test_non_allowlisted_reply_ignored(self):
        self.assertIsNone(self._poll(["U_OK"], reply_user="U_OTHER"))

    def test_allowlisted_reply_approves(self):
        self.assertEqual(self._poll(["U_OK"], reply_user="U_OK"), "approved")

    def test_empty_allowlist_allows_anyone(self):
        self.assertEqual(self._poll([], reply_user="U_ANY"), "approved")


if __name__ == "__main__":
    unittest.main()
