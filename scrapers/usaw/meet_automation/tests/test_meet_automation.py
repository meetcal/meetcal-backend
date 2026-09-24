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

    def test_candidate_list_is_bounded(self):
        # The meet page is third-party HTML; the candidate list declares a
        # ceiling instead of growing with whatever the page contains.
        html = "".join(
            f'<a href="https://cdn/{i}.pdf">doc {i}</a>'
            for i in range(detect.MAX_PDF_CANDIDATES + 50)
        )
        _, _, candidates = detect.discover_pdfs(html)
        self.assertEqual(len(candidates), detect.MAX_PDF_CANDIDATES)

    def test_classification_survives_the_ceiling(self):
        # A start list inside the bound is still classified even when the page
        # carries more links than the ceiling.
        html = '<a href="https://cdn/x_START_LIST.pdf">Start List</a>' + "".join(
            f'<a href="https://cdn/{i}.pdf">doc {i}</a>'
            for i in range(detect.MAX_PDF_CANDIDATES + 50)
        )
        start, _, candidates = detect.discover_pdfs(html)
        self.assertEqual(start, "https://cdn/x_START_LIST.pdf")
        self.assertEqual(len(candidates), detect.MAX_PDF_CANDIDATES)


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

    def test_approve_skips_an_unreadable_run(self):
        # One corrupt run directory used to raise out of the loop, so every
        # other pending approval on that tick was never published.
        args = pipeline.build_parser().parse_args(["approve", "--all-pending"])
        loaded = []

        def load(run_id):
            if run_id == "broken":
                raise ValueError("bundle.json is not JSON")
            loaded.append(run_id)
            return StagedBundle(run_id=run_id, watch_key="w", meet_name=MEET, status="ingested")

        with mock.patch.object(pipeline.stage, "list_runs", return_value=["broken", "fine"]), \
                mock.patch.object(pipeline.stage, "load_run", side_effect=load):
            self.assertEqual(pipeline.cmd_approve(args), 0)
        self.assertEqual(loaded, ["fine"])


class RunGuardTests(unittest.TestCase):
    """`run` must not stage (or mark-seen) a parse that produced 0 athletes
    unless --allow-empty is passed, so a broken parse is retried next run."""

    def _watch(self):
        return config.MeetWatch(key="k", meet_name=MEET, page_url="https://e.com")

    def _detect_result(self):
        return detect.DetectResult(
            watch_key="k",
            changed=True,
            start_list_url="https://e.com/s.pdf",
            schedule_url="https://e.com/c.pdf",
            reasons=["new"],
        )

    def _run(self, allow_empty: bool):
        args = pipeline.build_parser().parse_args(
            ["run", "--watch", "k", "--no-slack"] + (["--allow-empty"] if allow_empty else [])
        )
        calls = {"staged": False, "seen": False}

        def fake_write_run(*a, **k):
            calls["staged"] = True
            return Path(tempfile.gettempdir())

        with mock.patch.object(detect, "detect", return_value=self._detect_result()), \
            mock.patch.object(detect, "fetch_bytes", return_value=b""), \
            mock.patch.object(pipeline.scrape, "scrape", return_value=([], [], {})), \
            mock.patch.object(pipeline.stage, "write_run", side_effect=fake_write_run), \
            mock.patch.object(detect, "mark_seen",
                              side_effect=lambda *a, **k: calls.update(seen=True)):
            pipeline._run_one(self._watch(), args, SlackConfig())
        return calls

    def test_empty_parse_is_skipped_by_default(self):
        calls = self._run(allow_empty=False)
        self.assertFalse(calls["staged"])
        self.assertFalse(calls["seen"])

    def test_allow_empty_stages_anyway(self):
        calls = self._run(allow_empty=True)
        self.assertTrue(calls["staged"])
        self.assertTrue(calls["seen"])


class RunRequestTests(unittest.TestCase):
    """`/meet-run` drops a request file; `run --requested` drains it."""

    def _watches(self):
        return {
            "a": config.MeetWatch(key="a", meet_name="A", page_url="https://e.com/a"),
            "b": config.MeetWatch(key="b", meet_name="B", page_url="https://e.com/b"),
        }

    def test_resolve_single_key(self):
        watches, force = pipeline._resolve_run_request(
            {"key": "a", "all": False, "force": True}, "a", self._watches()
        )
        self.assertEqual([w.key for w in watches], ["a"])
        self.assertTrue(force)

    def test_resolve_all(self):
        watches, _ = pipeline._resolve_run_request(
            {"all": True}, "__all__", self._watches()
        )
        self.assertEqual({w.key for w in watches}, {"a", "b"})

    def test_resolve_falls_back_to_filename_stem(self):
        watches, _ = pipeline._resolve_run_request({}, "b", self._watches())
        self.assertEqual([w.key for w in watches], ["b"])

    def test_resolve_unknown_key_is_empty(self):
        watches, _ = pipeline._resolve_run_request(
            {"key": "gone"}, "gone", self._watches()
        )
        self.assertEqual(watches, [])

    def test_requested_drains_and_consumes(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            req_dir = tmp_path / config.RUN_REQUESTS_DIRNAME
            req_dir.mkdir()
            (req_dir / "a.json").write_text(json.dumps({"key": "a", "force": True}))
            (req_dir / "gone.json").write_text(json.dumps({"key": "gone"}))

            ran = []
            args = pipeline.build_parser().parse_args(["run", "--requested", "--no-slack"])
            original = config.STATE_DIR
            config.STATE_DIR = tmp_path
            try:
                with mock.patch.object(config, "load_watches", return_value=list(self._watches().values())), \
                    mock.patch.object(pipeline, "_run_one", side_effect=lambda w, a, c: ran.append(w.key)):
                    pipeline._run_requested(args, SlackConfig())
            finally:
                config.STATE_DIR = original

            self.assertEqual(ran, ["a"])  # only the real watch ran
            # Both request files are consumed, including the stale one, with no
            # leftover ".processing" claim files.
            self.assertEqual(list(req_dir.iterdir()), [])

    def test_unconsumable_request_is_skipped_not_rerun(self):
        """If the request file can't be claimed (e.g. owned by another user and
        the dir isn't writable), skip it — never run + re-run forever."""
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            req_dir = tmp_path / config.RUN_REQUESTS_DIRNAME
            req_dir.mkdir()
            (req_dir / "a.json").write_text(json.dumps({"key": "a"}))

            ran = []
            args = pipeline.build_parser().parse_args(["run", "--requested", "--no-slack"])
            original = config.STATE_DIR
            config.STATE_DIR = tmp_path
            try:
                with mock.patch.object(config, "load_watches", return_value=list(self._watches().values())), \
                    mock.patch.object(pipeline, "_run_one", side_effect=lambda w, a, c: ran.append(w.key)), \
                    mock.patch.object(Path, "rename", side_effect=PermissionError("EACCES")):
                    pipeline._run_requested(args, SlackConfig())
            finally:
                config.STATE_DIR = original

            self.assertEqual(ran, [])  # nothing ran — no wasted scrape, no loop
            self.assertTrue((req_dir / "a.json").exists())  # left for the operator to fix


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


class ThreadReplyBoundTests(unittest.TestCase):
    """`poll_approval` walks a third-party response; the scan is bounded and
    the bound is also requested from Slack."""

    def test_asks_slack_for_a_bounded_page_and_stops_at_the_bound(self):
        bundle = StagedBundle(
            run_id="r", watch_key="w", meet_name=MEET, slack=SlackRef(channel="C1", ts="root")
        )
        cfg = SlackConfig(bot_token="x")
        # The approving reply sits one past the ceiling, so a bounded scan must
        # not reach it.
        messages = [{"ts": "root"}] + [
            {"user": "U", "text": "noise"} for _ in range(slack.MAX_THREAD_REPLIES)
        ]
        messages.append({"user": "U", "text": "okay"})
        captured = {}

        class _Requests(_FakeRequests):
            def get(self, *a, **k):
                captured.update(k.get("params", {}))
                return _FakeResp({"ok": True, "messages": messages})

        with mock.patch.dict(sys.modules, {"requests": _Requests({})}):
            self.assertIsNone(slack.poll_approval(cfg, bundle))
        self.assertEqual(captured.get("limit"), slack.MAX_THREAD_REPLIES)


class IngestGuardTests(unittest.TestCase):
    def test_empty_meet_name_is_rejected(self):
        from usaw.meet_automation import ingest

        with self.assertRaisesRegex(ValueError, "meet_name is required"):
            ingest.ingest_bundle([], [], None, "")


if __name__ == "__main__":
    unittest.main()


class ApproveFailurePathTests(unittest.TestCase):
    """A failing ingest must park the run as failed and consume the decision
    file; otherwise the approve cron retries it (and spams Slack) forever."""

    def _run(self, tmp_path, ingest_side_effect, write_fails_for=None):
        config_original = config.STATE_DIR
        config.STATE_DIR = tmp_path
        decisions = tmp_path / "decisions"
        decisions.mkdir()
        for run_id in ("boom", "fine"):
            (decisions / f"{run_id}.json").write_text(json.dumps({"decision": "approved"}))
        bundles = {
            run_id: StagedBundle(
                run_id=run_id, watch_key="w", meet_name=MEET, status="pending_approval",
                slack=SlackRef(channel="C1", ts="1.2"),
            )
            for run_id in ("boom", "fine")
        }
        saved = []
        notified = []

        def do_ingest(bundle, replace, slack_cfg=None):
            if bundle.run_id == "boom":
                raise ingest_side_effect
            bundle.status = "ingested"
            return bundle

        def write_run(bundle):
            if bundle.run_id == write_fails_for and bundle.status == "ingested":
                raise OSError("read-only state dir")
            saved.append((bundle.run_id, bundle.status))

        args = pipeline.build_parser().parse_args(["approve", "--all-pending"])
        try:
            with mock.patch.object(pipeline.stage, "list_runs", return_value=["boom", "fine"]), \
                    mock.patch.object(pipeline.stage, "load_run", side_effect=lambda r: bundles[r]), \
                    mock.patch.object(pipeline.stage, "write_run", side_effect=write_run), \
                    mock.patch.object(pipeline, "_publish", side_effect=do_ingest), \
                    mock.patch.object(pipeline.slack, "post_thread_reply", side_effect=lambda cfg, b, t: notified.append((b.run_id, t))), \
                    mock.patch.object(pipeline.SlackConfig, "from_env", return_value=SlackConfig()):
                code = pipeline.cmd_approve(args)
        finally:
            config.STATE_DIR = config_original
        return code, bundles, saved, notified, decisions

    def test_failed_ingest_is_recorded_consumed_and_does_not_block_other_runs(self):
        with tempfile.TemporaryDirectory() as tmp:
            code, bundles, saved, notified, decisions = self._run(Path(tmp), RuntimeError("db down"))
            self.assertEqual(code, 1)
            self.assertEqual(bundles["boom"].status, "failed")
            self.assertIn(("boom", "failed"), saved)
            # The decision files are consumed either way, so the next tick
            # does not re-run the same approval.
            self.assertFalse((decisions / "boom.json").exists())
            self.assertFalse((decisions / "fine.json").exists())
            # The later run on the same tick still published.
            self.assertEqual(bundles["fine"].status, "ingested")
            failure_notes = [t for r, t in notified if r == "boom" and "failed to publish" in t]
            self.assertEqual(len(failure_notes), 1)
            # Driver text stays in the log, out of the channel.
            self.assertNotIn("db down", failure_notes[0])
            self.assertIn("RuntimeError", failure_notes[0])

    def test_published_run_whose_state_cannot_be_saved_is_not_reported_as_unwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            code, bundles, _, notified, decisions = self._run(
                Path(tmp), RuntimeError("db down"), write_fails_for="fine"
            )
            self.assertEqual(code, 1)
            # The data is live: the run is not parked as failed.
            self.assertEqual(bundles["fine"].status, "ingested")
            self.assertFalse((decisions / "fine.json").exists())
            fine_notes = [t for r, t in notified if r == "fine"]
            self.assertTrue(any("was published to Postgres" in t for t in fine_notes))
            self.assertFalse(any("nothing was written" in t for t in fine_notes))
            self.assertFalse(any("read-only state dir" in t for t in fine_notes))

    def test_failed_run_is_skipped_on_the_next_tick(self):
        with tempfile.TemporaryDirectory() as tmp:
            _, bundles, _, _, _ = self._run(Path(tmp), RuntimeError("db down"))
        bundles["boom"].status = "failed"
        args = pipeline.build_parser().parse_args(["approve", "--all-pending"])
        with mock.patch.object(pipeline.stage, "list_runs", return_value=["boom"]), \
                mock.patch.object(pipeline.stage, "load_run", return_value=bundles["boom"]), \
                mock.patch.object(pipeline, "_publish") as ingest, \
                mock.patch.object(pipeline.SlackConfig, "from_env", return_value=SlackConfig()):
            self.assertEqual(pipeline.cmd_approve(args), 0)
        ingest.assert_not_called()


class ReplyClassificationTests(unittest.TestCase):
    def _classify(self, text):
        cfg = SlackConfig()
        return slack.classify_reply(text, cfg.approve_words, cfg.reject_words)

    def test_negation_inside_an_approval_is_an_approval(self):
        self.assertEqual(self._classify("no issues, ship it"), "approved")
        self.assertEqual(self._classify("No problems. OK!"), "approved")

    def test_plain_keywords(self):
        self.assertEqual(self._classify("approve"), "approved")
        self.assertEqual(self._classify("okay"), "approved")
        self.assertEqual(self._classify("reject"), "rejected")
        self.assertEqual(self._classify("stop"), "rejected")
        self.assertEqual(self._classify("no"), "rejected")
        self.assertEqual(self._classify("No, redo it."), "rejected")

    def test_explicit_reject_wins_over_an_approve_word(self):
        self.assertEqual(self._classify("no, don't ship this"), "rejected")
        self.assertEqual(self._classify("ok but please reject, wrong PDF"), "rejected")

    def test_unrelated_chatter_is_not_a_decision(self):
        self.assertIsNone(self._classify("looking now"))
        self.assertIsNone(self._classify(""))
        self.assertIsNone(self._classify("there is no wso column, checking"))

    def test_poll_uses_the_classifier(self):
        bundle = StagedBundle(
            run_id="r", watch_key="w", meet_name=MEET, slack=SlackRef(channel="C1", ts="root")
        )
        data = {"ok": True, "messages": [{"ts": "root"}, {"user": "U", "text": "no issues, ship it"}]}
        with mock.patch.dict(sys.modules, {"requests": _FakeRequests(data)}):
            self.assertEqual(slack.poll_approval(SlackConfig(bot_token="x"), bundle), "approved")


class CaBundleTests(unittest.TestCase):
    def test_only_standard_variables_are_honoured(self):
        with mock.patch.dict("os.environ", {"CCR_CA_BUNDLE": "/sandbox.pem"}, clear=True):
            self.assertIs(detect._verify_arg(), True)
        with mock.patch.dict("os.environ", {"SSL_CERT_FILE": "/etc/ssl/ca.pem"}, clear=True):
            self.assertEqual(detect._verify_arg(), "/etc/ssl/ca.pem")
        with mock.patch.dict(
            "os.environ", {"REQUESTS_CA_BUNDLE": "/req.pem", "SSL_CERT_FILE": "/ssl.pem"}, clear=True
        ):
            self.assertEqual(detect._verify_arg(), "/req.pem")


class PlatformAndTimeValidationTests(unittest.TestCase):
    def test_platform_casing_is_normalised_before_the_check(self):
        report = validate(
            [_athlete(sessionPlatform="red")], [_session(platform="RED ")], MEET
        )
        codes = {f["code"] for f in report["findings"]}
        self.assertTrue(report["ok"], report["findings"])
        self.assertNotIn("platform_unknown", codes)
        self.assertNotIn("schedule_platform_unknown", codes)
        self.assertNotIn("athlete_session_not_in_schedule", codes)
        self.assertEqual(report["counts"]["platforms"], ["Red"])

    def test_unknown_platform_is_a_warning_not_an_error(self):
        # "Gold" is a real platform the app renders; the list only flags a
        # probable typo, so it must not block approval.
        report = validate(
            [_athlete(sessionPlatform="gold")], [_session(platform="GOLD ")], MEET
        )
        by_code = {f["code"]: f for f in report["findings"]}
        self.assertTrue(report["ok"], report["findings"])
        self.assertEqual(report["errors"], 0)
        self.assertEqual(by_code["platform_unknown"]["severity"], "warning")
        self.assertEqual(by_code["schedule_platform_unknown"]["severity"], "warning")
        self.assertEqual(by_code["platform_unknown"]["examples"], ["Jane Doe -> Gold"])
        self.assertEqual(by_code["schedule_platform_unknown"]["examples"], ["Gold"])
        # The athlete's canonicalised platform still pairs with the schedule row.
        self.assertNotIn("athlete_session_not_in_schedule", by_code)
        self.assertEqual(report["counts"]["platforms"], ["Gold"])

    def test_accepted_time_formats_do_not_warn(self):
        for start in ("09:00:00", "9:00", "9:00 AM", "14:30"):
            with self.subTest(start=start):
                report = validate([_athlete()], [_session(startTime=start)], MEET)
                self.assertNotIn(
                    "schedule_time_unparseable", {f["code"] for f in report["findings"]}
                )

    def test_unparseable_time_is_a_warning_not_an_error(self):
        report = validate(
            [_athlete()], [_session(startTime="after lunch", weighInTime="TBD")], MEET
        )
        by_code = {f["code"]: f for f in report["findings"]}
        self.assertTrue(report["ok"])
        self.assertEqual(by_code["schedule_time_unparseable"]["severity"], "warning")
        self.assertEqual(by_code["schedule_time_unparseable"]["count"], 2)
