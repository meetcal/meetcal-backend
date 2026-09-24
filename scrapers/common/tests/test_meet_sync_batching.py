"""The meet-sync scripts write each run through one batched ingest call.

``usaw/meet_to_supabase/scripts/sync-{meets,nat-meets,virus-meets}.js`` are run
for real under ``node``, with ``meet_sync_harness.js`` preloaded to stub the
Sport80 API and the Slack webhook, and ``$POSTGRES_INGEST_PYTHON`` pointed at
``fake_ingest.py``, which records every ingest / status-lookup call. The tests
check:

- one ``postgres_ingest.py --skip-errors scraperIngestion:ingestMeet`` call per
  run (and, for sync-meets.js, one status lookup), or a bounded number of
  chunks once a run is past ``INGEST_CHUNK_MAX_ROWS``;
- the old per-meet semantics: a bad meet is logged and skipped while the rest
  are written and announced, and the script still exits 0;
- a whole ingest call failing is logged per meet and exits 1 for cron, without
  losing the chunks that did succeed.

Needs ``node`` on PATH; skipped with a message otherwise.
"""

from __future__ import annotations

import json
import math
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

TESTS_DIR = Path(__file__).resolve().parent
SCRAPERS_DIR = TESTS_DIR.parents[1]
MEET_SYNC_DIR = SCRAPERS_DIR / "usaw" / "meet_to_supabase"
MEET_INGEST_JS = MEET_SYNC_DIR / "lib" / "meet_ingest.js"
HARNESS = TESTS_DIR / "meet_sync_harness.js"
FAKE_INGEST = TESTS_DIR / "fake_ingest.py"
INGEST_PY = SCRAPERS_DIR / "common" / "postgres_ingest.py"

SCRIPTS = ("sync-meets.js", "sync-nat-meets.js", "sync-virus-meets.js")
MEET_INGEST_ARGS = ["--skip-errors", "scraperIngestion:ingestMeet"]

NODE = shutil.which("node")


def raw_meet(name: str) -> dict:
    """A meet as the Sport80 widget API returns it."""
    return {
        "name": name,
        "address": "Iron Gym, 123 Main St, Springfield, Illinois, United States of America, 62701",
        "subtitle": "10\\/01\\/2026 - 10\\/02\\/2026",
    }


def python_cap(name: str) -> int:
    match = re.search(rf"^{name} = (.+)$", INGEST_PY.read_text(encoding="utf-8"), re.M)
    assert match is not None, name
    value = 1
    for factor in match.group(1).split("*"):
        value *= int(factor.strip())
    return value


def node_eval(source: str) -> object:
    result = subprocess.run(
        [NODE, "-e", source],
        capture_output=True,
        text=True,
        timeout=60,
        env={"PATH": os.environ.get("PATH", "")},
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return json.loads(result.stdout)


class Run:
    def __init__(self, completed: subprocess.CompletedProcess, calls: list[dict], slack: list[dict]):
        self.code = completed.returncode
        self.stdout = completed.stdout
        self.stderr = completed.stderr
        self.calls = calls
        self.slack = slack

    def ingest_calls(self) -> list[dict]:
        return [c for c in self.calls if c["script"] == "postgres_ingest.py"]

    def lookup_calls(self) -> list[dict]:
        return [c for c in self.calls if c["script"] == "lookup_meet_status.py"]

    def ingested_names(self) -> list[str]:
        return [row["name"] for call in self.ingest_calls() for row in call["payload"]]


@unittest.skipIf(NODE is None, "node is not on PATH; the meet-sync batching tests did not run")
class MeetSyncBatchingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="meet-sync-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.fake_python = self.tmp / "fake-python"
        self.fake_python.write_text(
            "#!/bin/sh\n"
            f"exec {shlex.quote(sys.executable)} {shlex.quote(str(FAKE_INGEST))} \"$@\"\n",
            encoding="utf-8",
        )
        self.fake_python.chmod(0o755)

    def run_script(self, script: str, meets: list[dict], **extra_env: str) -> Run:
        fixture = self.tmp / "meets.json"
        fixture.write_text(json.dumps(meets), encoding="utf-8")
        log = self.tmp / "ingest.log"
        slack = self.tmp / "slack.log"
        for path in (log, slack):
            path.write_text("", encoding="utf-8")
        env = {
            "PATH": os.environ.get("PATH", ""),
            "DATABASE_URL": "postgres://fake@127.0.0.1:1/none",
            "POSTGRES_INGEST_PYTHON": str(self.fake_python),
            "FAKE_INGEST_LOG": str(log),
            "MEET_SYNC_FIXTURE": str(fixture),
            "MEET_SYNC_SLACK_LOG": str(slack),
            "SLACK_WEBHOOK_URL": "https://hooks.invalid/meet-sync-test",
            **extra_env,
        }
        completed = subprocess.run(
            [NODE, "--require", str(HARNESS), str(MEET_SYNC_DIR / "scripts" / script)],
            cwd=self.tmp,
            capture_output=True,
            text=True,
            timeout=300,
            env=env,
        )
        calls = [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]
        posts = [json.loads(line) for line in slack.read_text(encoding="utf-8").splitlines()]
        return Run(completed, calls, posts)

    def chunk_max_rows(self) -> int:
        return node_eval(
            f"process.stdout.write(JSON.stringify(require({json.dumps(str(MEET_INGEST_JS))}).INGEST_CHUNK_MAX_ROWS))"
        )

    def test_each_script_writes_the_run_in_one_skipping_errors_call(self) -> None:
        names = ["Alpha Open", "EXISTING Beta Classic", "BAD Gamma Games", "Delta Open"]
        for script in SCRIPTS:
            with self.subTest(script=script):
                run = self.run_script(script, [raw_meet(n) for n in names])
                self.assertEqual(run.code, 0, run.stderr)
                [call] = run.ingest_calls()
                self.assertEqual(call["args"], MEET_INGEST_ARGS)
                self.assertEqual([row["name"] for row in call["payload"]], names)
                self.assertEqual(call["payload"][0]["venueState"], "IL")
                self.assertEqual(call["payload"][0]["startDate"], "2026-10-01")
                expected_lookups = 1 if script == "sync-meets.js" else 0
                self.assertEqual(len(run.lookup_calls()), expected_lookups)

                # Per-meet semantics: the bad meet is logged and skipped, the
                # others are written, and the run still succeeds.
                self.assertIn('Error ingesting "BAD Gamma Games": bad meet', run.stderr)
                self.assertIn("Failed to ingest 1 of 4 meets.", run.stderr)
                self.assertIn("Ingested: Alpha Open", run.stdout)
                self.assertIn("Ingested: Delta Open", run.stdout)
                self.assertNotIn("Ingested: EXISTING Beta Classic", run.stdout)
                self.assertIn("Sync completed. Processed 4 meets. Ingested: 2", run.stdout)
                [post] = run.slack
                self.assertIn("- Alpha Open\n- Delta Open", post["body"]["text"])
                self.assertTrue(post["body"]["text"].startswith("2 "))

    def test_sync_meets_skips_completed_meets_from_one_status_lookup(self) -> None:
        names = ["Alpha Open", "COMPLETED Winter Classic", "Delta Open"]
        run = self.run_script("sync-meets.js", [raw_meet(n) for n in names])
        self.assertEqual(run.code, 0, run.stderr)
        [lookup] = run.lookup_calls()
        self.assertEqual(lookup["payload"], {"names": names})
        self.assertIn("Skipping completed meet: COMPLETED Winter Classic", run.stdout)
        self.assertEqual(run.ingested_names(), ["Alpha Open", "Delta Open"])

    def test_a_failed_status_lookup_still_ingests_every_meet(self) -> None:
        names = ["Alpha Open", "COMPLETED Winter Classic"]
        run = self.run_script("sync-meets.js", [raw_meet(n) for n in names], FAKE_LOOKUP_FAIL="1")
        self.assertEqual(run.code, 0, run.stderr)
        self.assertIn("Status lookup (chunk 1/1, 2 meets) exited with code 1", run.stderr)
        self.assertEqual(run.ingested_names(), names)

    def test_a_large_run_is_chunked_under_the_stdin_caps(self) -> None:
        max_rows = self.chunk_max_rows()
        total = 2 * max_rows + 1
        names = [f"Meet {i:05d}" for i in range(total)]
        run = self.run_script("sync-meets.js", [raw_meet(n) for n in names])
        self.assertEqual(run.code, 0, run.stderr)
        chunks = run.ingest_calls()
        self.assertEqual(len(chunks), math.ceil(total / max_rows))
        self.assertEqual([len(c["payload"]) for c in chunks], [max_rows, max_rows, 1])
        self.assertEqual(run.ingested_names(), names)  # every meet once, in order
        self.assertTrue(all(c["args"] == MEET_INGEST_ARGS for c in chunks))
        self.assertTrue(all(c["bytes"] <= python_cap("MAX_STDIN_BYTES") for c in chunks))
        self.assertEqual(len(run.lookup_calls()), math.ceil(total / max_rows))
        self.assertIn(f"Ingested: {total}", run.stdout)

    def test_a_failed_ingest_call_exits_non_zero_and_keeps_the_other_chunks(self) -> None:
        max_rows = self.chunk_max_rows()
        names = [f"Meet {i:05d}" for i in range(max_rows)] + ["CRASH Meet", "Last Meet"]
        run = self.run_script("sync-nat-meets.js", [raw_meet(n) for n in names])
        self.assertEqual(run.code, 1, run.stderr)
        self.assertEqual(len(run.ingest_calls()), 2)
        self.assertIn(
            "Postgres ingest (chunk 2/2, 2 meets) exited with code 1: connection refused",
            run.stderr,
        )
        self.assertIn('Error ingesting "CRASH Meet": not written;', run.stderr)
        self.assertIn('Error ingesting "Last Meet": not written;', run.stderr)
        self.assertIn(f"Sync completed. Processed {len(names)} meets. Ingested: {max_rows}", run.stdout)
        # The first chunk was written, so its meets are still announced.
        [post] = run.slack
        self.assertTrue(post["body"]["text"].startswith(f"{max_rows} Nationals meets"))

    def test_a_single_chunk_failure_fails_the_run(self) -> None:
        run = self.run_script("sync-virus-meets.js", [raw_meet("CRASH Meet"), raw_meet("Other Meet")])
        self.assertEqual(run.code, 1)
        self.assertEqual(len(run.ingest_calls()), 1)
        self.assertIn('Error ingesting "Other Meet": not written;', run.stderr)
        self.assertEqual(run.slack, [])

    def test_no_meets_means_no_ingest_call(self) -> None:
        for script in SCRIPTS:
            with self.subTest(script=script):
                run = self.run_script(script, [])
                self.assertEqual(run.code, 0, run.stderr)
                self.assertEqual(run.calls, [])
                self.assertIn("No meets data found", run.stdout)


@unittest.skipIf(NODE is None, "node is not on PATH; the meet_ingest.js chunking tests did not run")
class MeetIngestChunkTests(unittest.TestCase):
    def chunk(self, items: list, max_rows: int, max_bytes: int) -> list[list]:
        return node_eval(
            f"const m = require({json.dumps(str(MEET_INGEST_JS))});"
            f"process.stdout.write(JSON.stringify(m.chunkForIngest({json.dumps(items)}, {max_rows}, {max_bytes})))"
        )

    def test_chunk_limits_sit_under_the_python_stdin_caps(self) -> None:
        values = node_eval(
            f"const m = require({json.dumps(str(MEET_INGEST_JS))});"
            "process.stdout.write(JSON.stringify([m.INGEST_STDIN_MAX_BYTES, m.INGEST_STDIN_MAX_ROWS,"
            " m.INGEST_CHUNK_MAX_BYTES, m.INGEST_CHUNK_MAX_ROWS]))"
        )
        stdin_bytes, stdin_rows, chunk_bytes, chunk_rows = values
        self.assertEqual(stdin_bytes, python_cap("MAX_STDIN_BYTES"))
        self.assertEqual(stdin_rows, python_cap("MAX_STDIN_ROWS"))
        self.assertLess(chunk_bytes, stdin_bytes)
        self.assertLess(chunk_rows, stdin_rows)

    def test_chunks_split_on_rows(self) -> None:
        self.assertEqual(self.chunk([1, 2, 3, 4, 5], 2, 1000), [[1, 2], [3, 4], [5]])
        self.assertEqual(self.chunk([], 2, 1000), [])

    def test_chunks_split_on_encoded_bytes(self) -> None:
        items = ["aaaa", "bbbb", "cccc"]  # each '"aaaa"' is 6 bytes
        # [a,b] encodes to 2 + 6 + 1 + 6 = 15 bytes; a third item would be 22.
        self.assertEqual(self.chunk(items, 100, 15), [["aaaa", "bbbb"], ["cccc"]])
        self.assertEqual(self.chunk(items, 100, 14), [["aaaa"], ["bbbb"], ["cccc"]])
        for chunk in self.chunk(items, 100, 15):
            self.assertLessEqual(len(json.dumps(chunk, separators=(",", ":")).encode()), 15)
        # Multi-byte characters count as bytes, not characters.
        self.assertEqual(self.chunk(["é", "é"], 100, 9), [["é"], ["é"]])
        # An item over the byte limit on its own still gets a chunk, alone.
        self.assertEqual(self.chunk(["x" * 50, "y"], 100, 10), [["x" * 50], ["y"]])


if __name__ == "__main__":
    unittest.main()
