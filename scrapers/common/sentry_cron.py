"""Sentry Crons check-ins for the host crontab jobs.

`run_scraper_job.sh` and `app/scripts/backup_db.sh` call this around each run:

    id="$(python3 sentry_cron.py start <slug> <crontab-match>)"
    ... job ...
    python3 sentry_cron.py finish <slug> "$id" <exit-code> <seconds> --log <path> --log-offset <bytes>

`start` upserts the monitor from the job's line in the installed crontab, so
Sentry also alerts when a job never runs or runs past MAX_RUNTIME_MINUTES.
`finish` closes the check-in as ok or error; on error it also sends an event
carrying this run's slice of the log, so the alert shows what broke.
`skipped` handles a run that found the previous one still holding the job
lock: an ok check-in while that run is younger than MAX_RUNTIME_MINUTES, and
an error check-in plus a "stuck" event once it is older, so skipped runs
never mark a hung job healthy.

`send_event` is also how scrapers/urlwatch/sentry_hooks.py reports page
changes.

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
# Sentry truncates an event message past 8192 characters.
MAX_MESSAGE_CHARS = 8000
# Cap on the urlwatch diff kept in an event's `extra`; envelopes over 1 MB are
# rejected outright.
MAX_EXTRA_CHARS = 16000
SLUG_RE = re.compile(r"^[a-z0-9_-]{1,50}$")
# Credentials a job may print on failure (a Slack error echoing its webhook
# URL, a connection string with a password). Scrubbed from the log tail.
SECRET_PATTERNS = (
    (re.compile(r"hooks\.slack\.com/services/\S+"), "hooks.slack.com/services/[redacted]"),
    (re.compile(r"xox[abprs]-[A-Za-z0-9-]+"), "xox?-[redacted]"),
    (re.compile(r"://[^/\s:@]+:[^/\s@]+@"), "://[redacted]@"),
)


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
    for prefix in ("posix/", "right/"):
        name = name.removeprefix(prefix)
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
        return redact(handle.read().decode("utf-8", errors="replace"))


def redact(text: str) -> str:
    for pattern, replacement in SECRET_PATTERNS:
        text = pattern.sub(replacement, text)
    return text


def send_event(
    dsn: Dsn,
    *,
    message: str,
    level: str,
    logger: str,
    fingerprint: list[str],
    tags: dict[str, str],
    extra: dict,
    contexts: dict | None = None,
) -> None:
    """Send one event through the envelope endpoint. `fingerprint` decides
    which Sentry issue it joins; `message` is the title plus body shown in
    the issue and its alert email."""
    event_id = uuid.uuid4().hex
    event = {
        "event_id": event_id,
        "timestamp": time.time(),
        "platform": "other",
        "level": level,
        "logger": logger,
        "environment": environment(),
        "server_name": socket.gethostname(),
        "message": {"formatted": message[:MAX_MESSAGE_CHARS]},
        "fingerprint": fingerprint,
        "tags": tags,
        "contexts": contexts or {},
        "extra": extra,
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


def send_failure_event(
    dsn: Dsn, slug: str, exit_code: int, duration: float, log_path: str | None, log_tail: str
) -> None:
    send_event(
        dsn,
        message=f"Cron job {slug} failed with exit code {exit_code}",
        level="error",
        logger="cron",
        fingerprint=["cron-job-failed", slug],
        tags={"monitor.slug": slug, "exit_code": str(exit_code)},
        contexts={"monitor": {"slug": slug}},
        extra={
            "log_tail": log_tail or "(no log output captured)",
            "log_path": log_path or "",
            "duration_seconds": round(duration, 1),
        },
    )


def with_schedule(payload: dict, match: str) -> dict:
    """Attach monitor_config from the job's crontab line. An unreadable crontab
    or no matching line still checks in, just without upserting the monitor."""
    try:
        schedule = find_schedule(read_crontab(), match)
    except (OSError, subprocess.SubprocessError) as error:
        warn(f"could not read the crontab ({error}); checking in without a schedule")
        return payload
    if schedule:
        return {**payload, "monitor_config": monitor_config(schedule)}
    warn(f"no crontab line matches {match!r}; checking in without a schedule")
    return payload


def start(dsn: Dsn, slug: str, match: str) -> str:
    check_in_id = uuid.uuid4().hex
    # Printed first: `finish` needs the id even if Sentry is unreachable now.
    print(check_in_id, flush=True)
    send_check_in(dsn, slug, with_schedule({"check_in_id": check_in_id, "status": "in_progress"}, match))
    return check_in_id


def skipped(dsn: Dsn, slug: str, match: str, holder_started_at: float) -> None:
    """`holder_started_at` is when the run holding the lock started (epoch
    seconds, 0 when unknown)."""
    running_minutes = (time.time() - holder_started_at) / 60 if holder_started_at > 0 else 0.0
    stuck = running_minutes > MAX_RUNTIME_MINUTES
    payload = {"check_in_id": uuid.uuid4().hex, "status": "error" if stuck else "ok"}
    try:
        send_check_in(dsn, slug, with_schedule(payload, match))
    finally:
        if stuck:
            send_event(
                dsn,
                message=(
                    f"Cron job {slug} has been running for {running_minutes:.0f} minutes; "
                    "later runs are skipped while it holds the job lock"
                ),
                level="error",
                logger="cron",
                fingerprint=["cron-job-stuck", slug],
                tags={"monitor.slug": slug},
                contexts={"monitor": {"slug": slug}},
                extra={"running_minutes": round(running_minutes)},
            )


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
        if name == "skipped":
            command.add_argument(
                "--holder-started-at",
                type=float,
                default=0.0,
                help="epoch seconds the lock holder started; 0 when unknown",
            )
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


def dsn_from_env() -> Dsn | None:
    """The configured DSN, or None when SENTRY_DSN is unset (reporting off)."""
    raw_dsn = os.getenv("SENTRY_DSN", "").strip()
    return Dsn(raw_dsn) if raw_dsn else None


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        dsn = dsn_from_env()
        if dsn is None:
            return 0
        if args.command == "start":
            start(dsn, args.slug, args.match)
        elif args.command == "skipped":
            skipped(dsn, args.slug, args.match, args.holder_started_at)
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
