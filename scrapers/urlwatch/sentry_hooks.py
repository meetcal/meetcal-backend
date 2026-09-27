"""urlwatch hooks: report each changed or failing page to Sentry.

Loaded by `run_scraper_job.sh urlwatch` through `urlwatch --hooks`, and
enabled by the `sentry` section of urlwatch.yaml. Each page becomes its own
Sentry event, sent with the helpers in scrapers/common/sentry_cron.py:

- changed: info level, one issue per distinct diff, so every change opens a
  new issue and triggers the project's "new urlwatch issue" alert. The diff
  is the message body, which the alert email shows.
- error: warning level, one issue per page, so a page that stays down does
  not alert again every night.
- new (first run for a page): info level, one issue per page.

With SENTRY_DSN unset nothing is sent. urlwatch has already saved every
page's new snapshot when reporters run, so a failed send is not retried next
night: each page is sent on its own, and any failure is raised after the rest
went out, so urlwatch exits non-zero and the cron failure event (with this
run's stdout report in its log tail) still carries the change.
"""

from __future__ import annotations

import hashlib

from urlwatch import reporters

from common import sentry_cron

LOGGER = "urlwatch"


def page_event(verb: str, name: str, location: str, content: str | None) -> dict:
    """Title, level and grouping for one urlwatch job state."""
    body = (content or "").strip()
    if verb == "changed":
        digest = hashlib.sha256(body.encode()).hexdigest()[:16]
        title, level, fingerprint = f"{name} changed", "info", [LOGGER, "changed", location, digest]
    elif verb == "error":
        title, level, fingerprint = f"{name} could not be checked", "warning", [LOGGER, "error", location]
    else:
        title, level, fingerprint = f"Now watching {name}", "info", [LOGGER, verb, location]
    message = f"{title}\n{location}"
    if body:
        message += f"\n\n{body}"
    return {
        "message": message,
        "level": level,
        "fingerprint": fingerprint,
        "tags": {"source": LOGGER, "urlwatch.verb": verb, "urlwatch.page": name[:200]},
        "extra": {"url": location, "content": body[-sentry_cron.MAX_EXTRA_CHARS :]},
    }


class SentryReporter(reporters.ReporterBase):
    """Send each changed or failing page to Sentry as its own event"""

    __kind__ = "sentry"

    def submit(self):
        dsn = sentry_cron.dsn_from_env()
        if dsn is None:
            return
        failures = []
        for job_state in self.report.get_filtered_job_states(self.job_states):
            if job_state.verb == "error":
                content = job_state.traceback
            elif job_state.verb == "changed":
                content = job_state.get_diff()
            else:
                content = None
            name = job_state.job.pretty_name()
            event = page_event(job_state.verb, name, job_state.job.get_location(), content)
            try:
                sentry_cron.send_event(dsn, logger=LOGGER, **event)
            except Exception as error:  # noqa: BLE001 - send the other pages first
                failures.append(f"{name}: {error}")
        if failures:
            raise RuntimeError("Sentry rejected urlwatch events: " + "; ".join(failures))
