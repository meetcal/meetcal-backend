# Meet Automation Pipeline

Automates the manual "get the start list + schedule PDFs → scrape → check →
upload" workflow for a USAW meet, with a **human approval gate** in Slack
before anything is written to the database.

```
watch a meet page
   │  (new/changed start-list + schedule PDFs?)
   ▼
download PDFs ──► scrape (reuses existing scrapers) ──► validate
   │                                                      │
   ▼                                                      ▼
stage to disk (no DB writes)  ◄───────────────  preview.html + report
   │
   ▼
Slack review message  ──►  you reply "okay"  ──►  ingest to Postgres + Convex
                            you reply "reject" ──► discard
```

Nothing is written to any database until you approve. This module **reuses**
the existing scrapers in `../final_start_scraper` and
`../owlcms_schedule_scraper` (it imports their extraction functions) and never
modifies them.

## Files

| File | Purpose |
| --- | --- |
| `config.py` | `MeetWatch` definitions, paths, Slack config from env |
| `detect.py` | Find + hash the start-list/schedule PDFs on a meet page; detect change |
| `scrape.py` | Drive the existing scrapers → canonical athlete + schedule rows |
| `validate.py` | Name/club/WSO/weight-class/session-coverage checks → report |
| `stage.py` / `models.py` | Persist a run (`bundle.json`) + `preview.html`; no DB writes |
| `preview.py` | Self-contained mobile-friendly HTML preview of staged data |
| `slack.py` | Block Kit review message + reply-based approval polling |
| `ingest.py` | **Dual-write** to Postgres (`common.postgres_writer`) + Convex (`/api/action`) |
| `pipeline.py` | CLI entry point |

Runtime state lives under `state/` (git-ignored): `state/seen.json` (last-seen
PDF hashes) and `state/runs/<run_id>/` (each staged run).

## Setup

```bash
cd scrapers/usaw/meet_automation
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
cp watches.example.json watches.json   # then edit URLs/meet name
```

`meet_name` in a watch **must** match the `meets.name` already synced by the
nightly `meet-sync` job, so the mobile API joins line up. Meet/venue details
are owned by `meet-sync`; this pipeline only stages athletes + schedule.

`watches.json` can be edited by hand or managed from Slack: the Rust API exposes
`POST /scrapers/slack/commands` backing `list` / `add` / `delete` slash
commands that edit this same file (see the repo README). Writes are atomic, so
the pipeline can read while a command updates it.

### Environment

```bash
DATABASE_URL=postgres://…            # Postgres ingest target
CONVEX_URL=…                         # Convex ingest target (phasing out)
SCRAPER_SECRET=…                     # required for Convex writes

# Slack — reply approval needs a bot token + channel:
SLACK_BOT_TOKEN=xoxb-…               # scopes: chat:write, channels:history (or groups:history)
SLACK_MEET_AUTOMATION_CHANNEL=C0123… # channel id the review is posted to
# …or notify-only via webhook (approval then falls back to the CLI):
SLACK_MEET_AUTOMATION_WEBHOOK_URL=https://hooks.slack.com/services/…

# Optional: makes the Slack "Preview the data" link clickable from your phone.
# Point this at wherever state/runs is served (e.g. a static file host / nginx).
MEET_AUTOMATION_PREVIEW_BASE_URL=https://meetcal.example.com/staged
```

Approval works three ways, in priority order:

1. **Buttons (recommended).** The review message carries *Approve & publish* /
   *Reject* buttons. A click hits the Rust API's
   `POST /scrapers/slack/interactions`, which writes a decision file under
   `MEET_AUTOMATION_STATE_DIR/decisions/<run_id>.json`. The `approve` cron
   consumes it and performs the dual-write. The API and this pipeline must share
   the same `MEET_AUTOMATION_STATE_DIR` (same server).
2. **Reply polling.** With a bot token, `approve` also reads thread replies for
   `okay` / `reject` (fallback when buttons aren't wired).
3. **CLI.** `pipeline.py ingest <run_id>` / `reject <run_id>` for manual control
   or webhook-only setups.

## Usage

```bash
export PYTHONPATH="$(git rev-parse --show-toplevel)/scrapers"
PY=.venv/bin/python

# 1. detect + scrape + validate + stage + Slack notify (NO db writes)
$PY -m usaw.meet_automation.pipeline run --all

# 2. poll Slack for your "okay"/"reject" reply; publish approved runs
$PY -m usaw.meet_automation.pipeline approve --all-pending

# inspect / operate manually
$PY -m usaw.meet_automation.pipeline list
$PY -m usaw.meet_automation.pipeline show <run_id>
$PY -m usaw.meet_automation.pipeline ingest <run_id>            # publish now (both targets)
$PY -m usaw.meet_automation.pipeline ingest <run_id> --target postgres
$PY -m usaw.meet_automation.pipeline reject <run_id>
$PY -m usaw.meet_automation.pipeline detect --all --candidates  # dry inspection
```

`run` only stages when a PDF is new or its content hash changed. Use `--force`
to stage regardless, `--no-slack` to skip notification.

### Cron wiring

Add to your scraper cron (or run via `run_scraper_job.sh`-style wrapper). The
two jobs are intentionally separate: `run` watches and stages; `approve` is a
short poll loop for your reply.

```cron
# stage + notify every morning
0 8 * * *   cd /path/meetcal-backend/scrapers && PYTHONPATH=. usaw/meet_automation/.venv/bin/python -m usaw.meet_automation.pipeline run --all   >> logs/meet-automation.log 2>&1
# act on your Slack reply every 5 minutes
*/5 * * * * cd /path/meetcal-backend/scrapers && PYTHONPATH=. usaw/meet_automation/.venv/bin/python -m usaw.meet_automation.pipeline approve --all-pending >> logs/meet-automation-approve.log 2>&1
```

## Testing without touching prod

Spin up a throwaway Postgres, load the schema + seed, and point `DATABASE_URL`
at it — never at prod. Two options:

**Docker** (per the repo README):

```bash
cd app/scripts && ./init_db.sh          # container `meetcal`, runs migrations
psql "$DATABASE_URL" -f seed_test_db.sql
```

**Local cluster** (no Docker):

```bash
initdb -D /tmp/pg/data -U postgres --auth=trust
pg_ctl -D /tmp/pg/data -o "-p 5433" start
createdb -p 5433 meetcal
for f in app/migrations/*.sql; do psql "host=127.0.0.1 port=5433 dbname=meetcal user=postgres" -f "$f"; done
psql "host=127.0.0.1 port=5433 dbname=meetcal user=postgres" -f app/scripts/seed_test_db.sql
export DATABASE_URL="postgres://postgres@127.0.0.1:5433/meetcal"
```

Then stage from a real PDF and ingest into the test DB only:

```bash
$PY -m usaw.meet_automation.pipeline run --watch 2026-nationals --no-slack --force
$PY -m usaw.meet_automation.pipeline ingest <run_id> --target postgres
```

Leave `CONVEX_URL` unset while testing so `--target postgres` (or `both`)
writes only to the local database.
