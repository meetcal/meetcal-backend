"""Sentry Crons check-ins for the host crontab jobs.

`run_scraper_job.sh` and `app/scripts/backup_db.sh` call this around each run:

    id="$(python3 sentry_cron.py start <slug> <crontab-match>)"
    ... job ...
    python3 sentry_cron.py finish <slug> "$id" <exit-code> <seconds> --log <path> --log-offset <bytes>

`start` upserts the monitor from the job's line in the installed crontab, so
Sentry also alerts when a job never runs or runs past MAX_RUNTIME_MINUTES.
`finish` closes the check-in as ok or error; on error it also sends an event
carrying this run's slice of the log, so the alert shows what broke.
`skipped` records an ok check-in for a run that found the previous one still
holding the job lock (the stuck run is caught by its own max_runtime).

Stdlib only: it runs under the system python3 before any venv exists. With
SENTRY_DSN unset every command is a no-op, and a Sentry failure is logged to
stderr but never changes the job's exit status.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.request
import uuid
from pathlib import Path
from urllib.parse import urlsplit

HTTP_TIMEOUT_SECONDS = 10
CRONTAB_TIMEOUT_SECONDS = 5
# Minutes after the scheduled time before Sentry marks a check-in missed.
CHECKIN_MARGIN_MINUTES = 5
# Minutes an in_progress check-in may stay open before Sentry marks it failed.
# The nightly scrapers finish well inside this; a hung job trips it.
MAX_RUNTIME_MINUTES = 120
# Jobs that fire more than hourly (`*/2`, `*/5`) open an issue only after this
# many failures in a row, so one flaky request does not page anyone.
FREQUENT_FAILURE_ISSUE_THRESHOLD = 3
# Bytes of this run's log sent with a failure event. Sentry trims longer
# strings in `extra`.
MAX_LOG_TAIL_BYTES = 8000
SLUG_RE = re.compile(r"^[a-z0-9_-]{1,50}$")


class Dsn:
    def __init__(self, dsn: str) -> None:
        parts = urlsplit(dsn)
        path, _, project_id = parts.path.rstrip("/").rpartition("/")
        if not parts.scheme or not parts.hostname or not parts.username or not project_id:
            raise ValueError("SENTRY_DSN is not a valid DSN")
        host = parts.hostname + (f":{parts.port}" if parts.port else "")
        self.key = parts.username
        self.api_base = f"{parts.scheme}://{host}{path}/api/{project_id}"

    def check_in_url(self, slug: str) -> str:
        return f"{self.api_base}/cron/{slug}/{self.key}/"

    def envelope_url(self) -> str:
        return f"{self.api_base}/envelope/"


def find_schedule(crontab: str, match: str) -> str | None:
    """Return the schedule of the first active crontab line whose command
    contains `match` as a whole trailing token (`... records` must not match
    `... records-extra`)."""
    pattern = re.compile(re.escape(match) + r"(?:\s|$)")
    for raw in crontab.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("@"):
            fields = line.split(None, 1)
            if len(fields) < 2:
                continue
            schedule, command = fields
        else:
            fields = line.split(None, 5)
            if len(fields) < 6:  # SHELL=/bin/bash and other env lines
                continue
            schedule, command = " ".join(fields[:5]), fields[5]
        if pattern.search(command):
            return schedule
    return None


def read_crontab() -> str:
    override = os.getenv("SENTRY_CRONS_CRONTAB")
    if override:
        return Path(override).read_text()
    result = subprocess.run(
        ["crontab", "-l"],
        capture_output=True,
        text=True,
        timeout=CRONTAB_TIMEOUT_SECONDS,
        check=False,
    )
    return result.stdout if result.returncode == 0 else ""


def local_timezone() -> str:
    tz = os.getenv("TZ", "").lstrip(":")
    if ("/" in tz and not tz.startswith("/")) or tz == "UTC":
        return tz
    try:
        name = Path("/etc/timezone").read_text().strip()
        if name:
            return name
    except OSError:
        pass
    try:
        target = os.readlink("/etc/localtime")
    except OSError:
        return "UTC"
    _, sep, name = target.partition("zoneinfo/")
    return name if sep and name else "UTC"


def monitor_config(schedule: str) -> dict:
    frequent = schedule.split()[0].startswith("*")
    return {
        "schedule": {"type": "crontab", "value": schedule},
        "timezone": local_timezone(),
        "checkin_margin": CHECKIN_MARGIN_MINUTES,
        "max_runtime": MAX_RUNTIME_MINUTES,
        "failure_issue_threshold": FREQUENT_FAILURE_ISSUE_THRESHOLD if frequent else 1,
        "recovery_threshold": 1,
    }


def environment() -> str:
    return os.getenv("SENTRY_ENVIRONMENT") or "production"


def _post(url: str, body: bytes, headers: dict[str, str]) -> None:
    request = urllib.request.Request(url, data=body, headers=headers, method="POST")
    with urllib.request.urlopen(request, timeout=HTTP_TIMEOUT_SECONDS) as response:
        response.read()


def send_check_in(dsn: Dsn, slug: str, payload: dict) -> None:
    body = json.dumps({**payload, "environment": environment()}).encode()
    _post(dsn.check_in_url(slug), body, {"Content-Type": "application/json"})


def read_log_slice(path: str | None, offset: int) -> str:
    """This run's output: the log bytes appended since `offset`, last
    MAX_LOG_TAIL_BYTES only. Empty when stdout was not a regular file."""
    if not path or not os.path.isfile(path):
        return ""
    with open(path, "rb") as handle:
        size = handle.seek(0, os.SEEK_END)
        if offset < 0 or offset > size:  # rotated or truncated mid-run
            offset = 0
        handle.seek(max(offset, size - MAX_LOG_TAIL_BYTES))
        return handle.read().decode("utf-8", errors="replace")


def send_failure_event(
    dsn: Dsn, slug: str, exit_code: int, duration: float, log_path: str | None, log_tail: str
) -> None:
    event_id = uuid.uuid4().hex
    event = {
        "event_id": event_id,
        "timestamp": time.time(),
        "platform": "other",
        "level": "error",
        "logger": "cron",
        "environment": environment(),
        "server_name": socket.gethostname(),
        "message": {"formatted": f"Cron job {slug} failed with exit code {exit_code}"},
        "fingerprint": ["cron-job-failed", slug],
        "tags": {"monitor.slug": slug, "exit_code": str(exit_code)},
        "contexts": {"monitor": {"slug": slug}},
        "extra": {
            "log_tail": log_tail or "(no log output captured)",
            "log_path": log_path or "",
            "duration_seconds": round(duration, 1),
        },
    }
    envelope = b"\n".join(
        json.dumps(item).encode()
        for item in ({"event_id": event_id}, {"type": "event"}, event)
    )
    auth = f"Sentry sentry_version=7, sentry_key={dsn.key}, sentry_client=meetcal-sentry-cron/1.0"
    _post(
        dsn.envelope_url(),
        envelope,
        {"Content-Type": "application/x-sentry-envelope", "X-Sentry-Auth": auth},
    )


def start(dsn: Dsn, slug: str, match: str) -> str:
    check_in_id = uuid.uuid4().hex
    # Printed first: `finish` needs the id even if Sentry is unreachable now.
    print(check_in_id, flush=True)
    payload: dict = {"check_in_id": check_in_id, "status": "in_progress"}
    schedule = find_schedule(read_crontab(), match)
    if schedule:
        payload["monitor_config"] = monitor_config(schedule)
    else:
        warn(f"no crontab line matches {match!r}; checking in without a schedule")
    send_check_in(dsn, slug, payload)
    return check_in_id


def skipped(dsn: Dsn, slug: str, match: str) -> None:
    payload: dict = {"check_in_id": uuid.uuid4().hex, "status": "ok"}
    schedule = find_schedule(read_crontab(), match)
    if schedule:
        payload["monitor_config"] = monitor_config(schedule)
    send_check_in(dsn, slug, payload)


def finish(
    dsn: Dsn,
    slug: str,
    check_in_id: str,
    exit_code: int,
    duration: float,
    log_path: str | None,
    log_offset: int,
) -> None:
    status = "ok" if exit_code == 0 else "error"
    payload = {
        "check_in_id": check_in_id or uuid.uuid4().hex,
        "status": status,
        "duration": max(duration, 0.0),
    }
    try:
        send_check_in(dsn, slug, payload)
    finally:
        if status == "error":
            send_failure_event(
                dsn, slug, exit_code, duration, log_path, read_log_slice(log_path, log_offset)
            )


def warn(message: str) -> None:
    print(f"sentry_cron: {message}", file=sys.stderr, flush=True)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("start", "skipped"):
        command = commands.add_parser(name)
        command.add_argument("slug")
        command.add_argument("match", help="text identifying the job's crontab line")
    done = commands.add_parser("finish")
    done.add_argument("slug")
    done.add_argument("check_in_id")
    done.add_argument("exit_code", type=int)
    done.add_argument("duration", type=float, help="seconds")
    done.add_argument("--log")
    done.add_argument("--log-offset", type=int, default=0)
    args = parser.parse_args(argv)
    if not SLUG_RE.match(args.slug):
        parser.error(f"invalid monitor slug: {args.slug}")
    return args


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    raw_dsn = os.getenv("SENTRY_DSN", "").strip()
    if not raw_dsn:
        return 0
    try:
        dsn = Dsn(raw_dsn)
        if args.command == "start":
            start(dsn, args.slug, args.match)
        elif args.command == "skipped":
            skipped(dsn, args.slug, args.match)
        else:
            finish(
                dsn,
                args.slug,
                args.check_in_id,
                args.exit_code,
                args.duration,
                args.log,
                args.log_offset,
            )
    except Exception as error:  # noqa: BLE001 - reporting must never fail the job
        warn(f"{args.command} {args.slug} failed: {error}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
