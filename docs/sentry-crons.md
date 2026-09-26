# Sentry Crons

Every host cron job checks in with [Sentry Crons](https://docs.sentry.io/product/crons/):
the 17 `scrapers/run_scraper_job.sh <job>` jobs (monitor slug = job name) and
`app/scripts/backup_db.sh` (slug `postgres-backup`). Sentry opens an issue when a job

- **fails**: non-zero exit. A second event, `Cron job <slug> failed with exit code N`,
  carries this run's last 8 KB of log output under **Additional Data → log_tail**.
- **is missed**: no check-in within 5 minutes of its crontab time (host down, cron
  stopped, `.env` unreadable).
- **hangs**: still running after 120 minutes.

Multi-step jobs (`wso-records`, `entries`, `meet-sync`) run every step even when one
fails, then exit non-zero listing the failed steps, so one broken WSO sheet or entry
page still fails the job without skipping the rest.

Jobs that fire more than hourly (`*/2`, `*/5`) open an issue only after 3 failures in
a row. A run that finds the previous one still holding the job lock checks in `ok`, so
a long `meet-automation-requests` run does not count as missed.

## Setup

1. In Sentry, create a project (for example `meetcal-backend`, platform Python) and copy
   its DSN from **Settings → Client Keys**.
2. Add it to the backend `.env` on the cron host:

   ```sh
   SENTRY_DSN=https://<key>@o<org>.ingest.us.sentry.io/<project-id>
   # SENTRY_ENVIRONMENT=production   # default
   ```

3. Nothing else: the next run of each job creates its monitor. Monitors copy the
   schedule from that job's line in the installed crontab (`crontab -l`) and the
   host timezone, so change a schedule by editing
   [`app/deploy/meetcal-scrapers.cron`](../app/deploy/meetcal-scrapers.cron) and
   reinstalling it.
4. In Sentry, add an alert (email, Slack) for the project's issues and the cron
   monitors.

Without `SENTRY_DSN` the check-ins are a no-op. A Sentry outage prints a
`sentry_cron:` warning into the job log and never changes the job's exit status.

The helper is [`scrapers/common/sentry_cron.py`](../scrapers/common/sentry_cron.py)
(stdlib only, system `python3`). Tests: `scrapers/common/tests/test_sentry_cron.py`.
