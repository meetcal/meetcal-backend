"""Sentry Crons helper: DSN parsing, crontab schedule lookup, and the payloads
sent for start / finish / skipped. No network: `_post` is patched."""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from common import sentry_cron

DSN = "https://abc123@o42.ingest.us.sentry.io/7"
ROOT = "/home/maddisen/dev/meetcal-llc/meetcal-backend"
CRONTAB = f"""# Nightly scraper jobs
SHELL=/bin/bash
MEETCAL_BACKEND_ROOT={ROOT}

0 3 * * * {ROOT}/app/scripts/backup_db.sh >> {ROOT}/app/backups/meetcal-backup.log 2>&1
35 23 * * * {ROOT}/scrapers/run_scraper_job.sh records >> {ROOT}/scrapers/logs/records.log 2>&1
55 23 * * * {ROOT}/scrapers/run_scraper_job.sh umwf-records >> {ROOT}/scrapers/logs/umwf-records.log 2>&1
# 0 9 * * * {ROOT}/scrapers/run_scraper_job.sh standards
*/2 * * * * {ROOT}/scrapers/run_scraper_job.sh meet-automation-requests >> /tmp/x.log 2>&1
@weekly {ROOT}/scrapers/run_scraper_job.sh weekly-job
"""


class DsnTests(unittest.TestCase):
    def test_builds_check_in_and_envelope_urls(self) -> None:
        dsn = sentry_cron.Dsn(DSN)
        self.assertEqual(
            dsn.check_in_url("records"),
            "https://o42.ingest.us.sentry.io/api/7/cron/records/abc123/",
        )
        self.assertEqual(dsn.envelope_url(), "https://o42.ingest.us.sentry.io/api/7/envelope/")

    def test_keeps_self_hosted_port_and_path_prefix(self) -> None:
        dsn = sentry_cron.Dsn("http://key@sentry.local:9000/prefix/3")
        self.assertEqual(dsn.envelope_url(), "http://sentry.local:9000/prefix/api/3/envelope/")

    def test_rejects_dsn_without_key_or_project(self) -> None:
        for bad in ("https://o42.ingest.sentry.io/7", "https://abc@o42.ingest.sentry.io/", "nonsense"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                sentry_cron.Dsn(bad)


class FindScheduleTests(unittest.TestCase):
    def test_matches_the_job_line(self) -> None:
        self.assertEqual(sentry_cron.find_schedule(CRONTAB, "run_scraper_job.sh records"), "35 23 * * *")
        self.assertEqual(sentry_cron.find_schedule(CRONTAB, "backup_db.sh"), "0 3 * * *")
        self.assertEqual(
            sentry_cron.find_schedule(CRONTAB, "run_scraper_job.sh meet-automation-requests"),
            "*/2 * * * *",
        )

    def test_job_name_must_be_a_whole_token(self) -> None:
        self.assertIsNone(sentry_cron.find_schedule(CRONTAB, "run_scraper_job.sh umwf"))
        self.assertIsNone(sentry_cron.find_schedule(CRONTAB, "run_scraper_job.sh meet-automation"))

    def test_ignores_comments_and_env_lines(self) -> None:
        self.assertIsNone(sentry_cron.find_schedule(CRONTAB, "run_scraper_job.sh standards"))
        self.assertIsNone(sentry_cron.find_schedule(CRONTAB, "/bin/bash"))

    def test_supports_macro_schedules(self) -> None:
        self.assertEqual(sentry_cron.find_schedule(CRONTAB, "run_scraper_job.sh weekly-job"), "@weekly")


class MonitorConfigTests(unittest.TestCase):
    def test_frequent_jobs_need_consecutive_failures(self) -> None:
        with mock.patch.dict(os.environ, {"TZ": "America/New_York"}):
            nightly = sentry_cron.monitor_config("35 23 * * *")
            frequent = sentry_cron.monitor_config("*/2 * * * *")
        self.assertEqual(nightly["failure_issue_threshold"], 1)
        self.assertEqual(frequent["failure_issue_threshold"], sentry_cron.FREQUENT_FAILURE_ISSUE_THRESHOLD)
        self.assertEqual(nightly["timezone"], "America/New_York")
        self.assertEqual(nightly["schedule"], {"type": "crontab", "value": "35 23 * * *"})


class CommandTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        crontab = Path(self.tmp.name) / "crontab"
        crontab.write_text(CRONTAB)
        env = {"SENTRY_DSN": DSN, "SENTRY_CRONS_CRONTAB": str(crontab), "TZ": "America/New_York"}
        env_patch = mock.patch.dict(os.environ, env)
        env_patch.start()
        self.addCleanup(env_patch.stop)
        os.environ.pop("SENTRY_ENVIRONMENT", None)
        post_patch = mock.patch.object(sentry_cron, "_post")
        self.post = post_patch.start()
        self.addCleanup(post_patch.stop)

    def sent(self) -> list[tuple[str, list[dict]]]:
        calls = []
        for call in self.post.call_args_list:
            url, body, _headers = call.args
            calls.append((url, [json.loads(line) for line in body.decode().split("\n")]))
        return calls

    def test_start_upserts_monitor_and_prints_id(self) -> None:
        with mock.patch("builtins.print") as printed:
            sentry_cron.main(["start", "records", "run_scraper_job.sh records"])
        check_in_id = printed.call_args.args[0]
        [(url, [body])] = self.sent()
        self.assertTrue(url.endswith("/api/7/cron/records/abc123/"))
        self.assertEqual(body["check_in_id"], check_in_id)
        self.assertEqual(body["status"], "in_progress")
        self.assertEqual(body["environment"], "production")
        self.assertEqual(body["monitor_config"]["schedule"]["value"], "35 23 * * *")

    def test_start_without_crontab_line_still_checks_in(self) -> None:
        with mock.patch("builtins.print"), mock.patch.object(sentry_cron, "warn") as warned:
            sentry_cron.main(["start", "manual", "run_scraper_job.sh manual"])
        [(_url, [body])] = self.sent()
        self.assertNotIn("monitor_config", body)
        warned.assert_called_once()

    def test_finish_ok_sends_only_the_check_in(self) -> None:
        sentry_cron.main(["finish", "records", "id1", "0", "12.5"])
        [(url, [body])] = self.sent()
        self.assertIn("/cron/records/", url)
        self.assertEqual(body, {"check_in_id": "id1", "status": "ok", "duration": 12.5, "environment": "production"})

    def test_finish_error_sends_event_with_this_runs_log(self) -> None:
        log = Path(self.tmp.name) / "records.log"
        log.write_text("yesterday's run\n")
        offset = log.stat().st_size
        with log.open("a") as handle:
            handle.write("Traceback (most recent call last):\nValueError: boom\n")
        sentry_cron.main(
            ["finish", "records", "id1", "3", "40", "--log", str(log), "--log-offset", str(offset)]
        )
        [(check_in_url, [check_in]), (event_url, [header, item, event])] = self.sent()
        self.assertIn("/cron/records/", check_in_url)
        self.assertEqual(check_in["status"], "error")
        self.assertTrue(event_url.endswith("/api/7/envelope/"))
        self.assertEqual(header["event_id"], event["event_id"])
        self.assertEqual(item, {"type": "event"})
        self.assertEqual(event["fingerprint"], ["cron-job-failed", "records"])
        self.assertEqual(event["tags"]["exit_code"], "3")
        self.assertIn("ValueError: boom", event["extra"]["log_tail"])
        self.assertNotIn("yesterday", event["extra"]["log_tail"])
        auth = self.post.call_args_list[1].args[2]["X-Sentry-Auth"]
        self.assertIn("sentry_key=abc123", auth)

    def test_finish_error_still_reports_when_check_in_fails(self) -> None:
        self.post.side_effect = [OSError("network down"), None]
        with mock.patch.object(sentry_cron, "warn"):
            self.assertEqual(sentry_cron.main(["finish", "records", "", "1", "1"]), 0)
        self.assertEqual(self.post.call_count, 2)
        _url, [check_in] = self.sent()[0]
        self.assertTrue(check_in["check_in_id"])

    def test_skipped_records_an_ok_check_in(self) -> None:
        sentry_cron.main(["skipped", "meet-automation-requests", "run_scraper_job.sh meet-automation-requests"])
        [(_url, [body])] = self.sent()
        self.assertEqual(body["status"], "ok")
        self.assertEqual(body["monitor_config"]["failure_issue_threshold"], 3)

    def test_without_dsn_nothing_is_sent(self) -> None:
        os.environ.pop("SENTRY_DSN")
        with mock.patch("builtins.print") as printed:
            sentry_cron.main(["start", "records", "run_scraper_job.sh records"])
        sentry_cron.main(["finish", "records", "", "1", "1"])
        printed.assert_not_called()
        self.post.assert_not_called()

    def test_errors_never_change_the_exit_status(self) -> None:
        self.post.side_effect = OSError("network down")
        with mock.patch("builtins.print"), mock.patch.object(sentry_cron, "warn") as warned:
            self.assertEqual(sentry_cron.main(["start", "records", "run_scraper_job.sh records"]), 0)
        warned.assert_called_once()


class LogSliceTests(unittest.TestCase):
    def test_truncated_log_falls_back_to_the_start(self) -> None:
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as handle:
            handle.write("fresh\n")
        self.addCleanup(os.unlink, handle.name)
        self.assertEqual(sentry_cron.read_log_slice(handle.name, 10_000), "fresh\n")

    def test_keeps_only_the_last_bytes(self) -> None:
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as handle:
            handle.write("x" * (sentry_cron.MAX_LOG_TAIL_BYTES + 100) + "END")
        self.addCleanup(os.unlink, handle.name)
        tail = sentry_cron.read_log_slice(handle.name, 0)
        self.assertEqual(len(tail), sentry_cron.MAX_LOG_TAIL_BYTES)
        self.assertTrue(tail.endswith("END"))

    def test_non_file_stdout_sends_nothing(self) -> None:
        self.assertEqual(sentry_cron.read_log_slice("/dev/null", 0), "")
        self.assertEqual(sentry_cron.read_log_slice(None, 0), "")


if __name__ == "__main__":
    unittest.main()
