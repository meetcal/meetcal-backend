"""urlwatch -> Sentry reporter (scrapers/urlwatch/sentry_hooks.py). urlwatch
itself is not installed in CI, so a stub `urlwatch.reporters` stands in for
the base class; `send_event` is patched, so nothing leaves the process."""

from __future__ import annotations

import importlib.util
import os
import sys
import types
import unittest
from pathlib import Path
from unittest import mock

from common import sentry_cron

HOOKS_PATH = Path(__file__).resolve().parents[2] / "urlwatch" / "sentry_hooks.py"


def load_hooks():
    reporters = types.ModuleType("urlwatch.reporters")

    class ReporterBase:
        def __init__(self, report, config, job_states, duration):
            self.report = report
            self.job_states = job_states

    reporters.ReporterBase = ReporterBase
    package = types.ModuleType("urlwatch")
    package.reporters = reporters
    with mock.patch.dict(sys.modules, {"urlwatch": package, "urlwatch.reporters": reporters}):
        spec = importlib.util.spec_from_file_location("sentry_hooks_under_test", HOOKS_PATH)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    return module


def job_state(verb: str, name: str, url: str, *, diff: str = "", traceback: str = ""):
    job = types.SimpleNamespace(pretty_name=lambda: name, get_location=lambda: url)
    return types.SimpleNamespace(verb=verb, job=job, get_diff=lambda: diff, traceback=traceback)


class PageEventTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.hooks = load_hooks()

    def test_each_distinct_change_is_its_own_issue(self) -> None:
        first = self.hooks.page_event("changed", "Events", "https://x/events", "+Camp Oct 16")
        second = self.hooks.page_event("changed", "Events", "https://x/events", "+Camp Oct 23")
        self.assertEqual(first["level"], "info")
        self.assertEqual(first["fingerprint"][:3], ["urlwatch", "changed", "https://x/events"])
        self.assertNotEqual(first["fingerprint"], second["fingerprint"])
        self.assertTrue(first["message"].startswith("Events changed\nhttps://x/events\n\n+Camp Oct 16"))
        self.assertEqual(first["tags"]["source"], "urlwatch")

    def test_a_page_that_stays_down_is_one_issue(self) -> None:
        monday = self.hooks.page_event("error", "Results", "https://x/results", "Timeout A")
        tuesday = self.hooks.page_event("error", "Results", "https://x/results", "Timeout B")
        self.assertEqual(monday["level"], "warning")
        self.assertEqual(monday["fingerprint"], tuesday["fingerprint"])
        self.assertTrue(monday["message"].startswith("Results could not be checked"))

    def test_first_sight_of_a_page(self) -> None:
        event = self.hooks.page_event("new", "Records", "https://x/records", None)
        self.assertEqual(event["message"], "Now watching Records\nhttps://x/records")


class ReporterTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.hooks = load_hooks()

    def submit(self, states, env):
        report = types.SimpleNamespace(get_filtered_job_states=lambda states: iter(states))
        reporter = self.hooks.SentryReporter(report, {}, states, None)
        with mock.patch.dict(os.environ, env, clear=False), mock.patch.object(
            sentry_cron, "send_event"
        ) as send:
            reporter.submit()
        return send

    def test_sends_one_event_per_reported_page(self) -> None:
        states = [
            job_state("changed", "Events", "https://x/events", diff="+Camp"),
            job_state("error", "Results", "https://x/results", traceback="Timeout"),
        ]
        send = self.submit(states, {"SENTRY_DSN": "https://k@o1.ingest.us.sentry.io/2"})
        self.assertEqual(send.call_count, 2)
        messages = [call.kwargs["message"] for call in send.call_args_list]
        self.assertIn("+Camp", messages[0])
        self.assertIn("Timeout", messages[1])
        self.assertEqual({call.kwargs["logger"] for call in send.call_args_list}, {"urlwatch"})

    def test_without_dsn_nothing_is_sent(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("SENTRY_DSN", None)
            send = self.submit([job_state("changed", "Events", "https://x/events", diff="+x")], {})
        send.assert_not_called()


if __name__ == "__main__":
    unittest.main()
