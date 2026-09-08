# Meet Automation — End-to-End Setup Runbook

Everything needed to stand up the Slack-driven meet pipeline and the entries
manager on a self-hosted server. Two cooperating pieces:

- **The Rust API** (always-on) hosts the Slack control surfaces. It edits JSON
  files on the server's disk and drops approval decisions. No DB writes.
- **The Python pipeline** (cron) watches meet pages, scrapes, validates, posts
  Slack reviews, and on approval writes to Postgres.

They communicate through files on the **same server**: `watches.json`,
`entries_targets.json`, and `state/` (staged runs + approval decisions). Slack
edits take effect with **no redeploy or git pull**.

```
Slack ─slash cmd─▶ Rust API ─writes─▶ watches.json / entries_targets.json ─read─▶ cron jobs
Slack ─/meet-run─▶ Rust API ─writes─▶ state/run_requests/<key>.json ─read─▶ run --requested cron ─▶ Slack review
Slack ─button────▶ Rust API ─writes─▶ state/decisions/<run>.json ─read─▶ approve cron ─▶ Postgres
```

---

## 1. Prerequisites (server)

- Rust (stable) + `sqlx-cli`
- Python 3.11+
- PostgreSQL 16
- Node.js (for the entries scraper)
- `tesseract` (optional — only for image-only schedule PDFs)
- A public HTTPS URL for the API (Slack must reach it). Put it behind the
  existing Caddy/nginx config in [`app/deploy/`](../app/deploy/).

## 2. Clone + environment

```bash
git clone <repo> meetcal-backend && cd meetcal-backend
cp .env.example .env   # fill in values (see §7)
```

## 3. Database

```bash
cd app/scripts && ./init_db.sh        # Docker Postgres + migrations
# or apply app/migrations/*.sql to an existing Postgres, in filename order
```

For a throwaway test DB, also load `app/scripts/seed_test_db.sql`. **Never point
the pipeline's `DATABASE_URL` at prod while testing** — use a Docker/local DB.

## 4. Build & run the API

```bash
cd app && cargo build --release
# run via the provided systemd unit:
#   app/deploy/meetcal-api.service   (sets env, runs the release binary)
```

The API listens on `application_port` (default 3000). Health check: `GET /health`.

## 5. Slack app setup

Create a Slack app (https://api.slack.com/apps) for your workspace:

1. **Basic Information → Signing Secret** → set as `SLACK_SIGNING_SECRET`.
   Both Slack endpoints stay disabled (HTTP 503) until this is set.
2. **OAuth & Permissions → Bot Token Scopes**: `chat:write`, and
   `channels:history` (public channel) or `groups:history` (private). Install
   to the workspace; copy the **Bot User OAuth Token** → `SLACK_BOT_TOKEN`.
   (The bot posts review messages and reads thread replies / posts confirmations.)
3. **Slash Commands** — create these, all with Request URL
   `https://<your-api>/scrapers/slack/commands`:
   - `/meet-list`, `/meet-add`, `/meet-delete`, `/meet-run`
   - `/entries-list`, `/entries-add`, `/entries-delete`
4. **Interactivity & Shortcuts** — turn **on**; Request URL
   `https://<your-api>/scrapers/slack/interactions` (powers the Approve/Reject
   buttons).
5. **Invite the bot** to the channel(s) you'll use.

### One channel or two?

Commands route by **command name** (`/meet-*` vs `/entries-*`), so you can run
everything in **one channel** or split meets and entries across two — your call:

- **One channel:** set `SLACK_MEET_AUTOMATION_CHANNEL` to it (leave
  `SLACK_ENTRIES_CHANNEL` unset, or set it to the same id).
- **Two channels:** set both vars. Both are allowed; the command name still
  decides the list.
- **No restriction:** leave both unset (any channel the bot is in).

## 6. Python pipeline setup

```bash
cd scrapers/usaw/meet_automation
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
# Seed an initial watch list (or do it from Slack with /meet-add):
cp watches.example.json watches.json
```

`meet_name` in a watch **must** equal the `meets.name` synced by the nightly
`meet-sync` job, so the mobile API joins line up.

## 7. Environment variables

| Variable | Used by | Purpose |
| --- | --- | --- |
| `DATABASE_URL` | pipeline | Postgres ingest target (point at TEST db while testing) |
| `SLACK_SIGNING_SECRET` | API | Verifies Slack requests; **required to enable both endpoints** |
| `SLACK_BOT_TOKEN` | pipeline | Posts reviews, reads replies, posts confirmations |
| `SLACK_MEET_AUTOMATION_CHANNEL` | API, pipeline | Channel for meet reviews / meet commands |
| `SLACK_ENTRIES_CHANNEL` | API | Channel allowlisted for entries commands (optional) |
| `MEET_AUTOMATION_SLACK_ALLOWED_USERS` | API | Optional user allowlist (comma/space separated) |
| `MEET_AUTOMATION_WATCHES_PATH` | API, pipeline | Path to `watches.json` (must match on both) |
| `ENTRIES_TARGETS_PATH` | API, entries cron | Path to `entries_targets.json` (must match) |
| `MEET_AUTOMATION_STATE_DIR` | API, pipeline | Staged runs + approval decisions (must match) |
| `MEET_AUTOMATION_PREVIEW_BASE_URL` | pipeline | Makes the Slack "Preview" link open from your phone |

> **Shared paths:** if the API and the scrapers are separate checkouts on the
> box, set `MEET_AUTOMATION_WATCHES_PATH`, `ENTRIES_TARGETS_PATH`, and
> `MEET_AUTOMATION_STATE_DIR` to the **same absolute locations** in both
> environments. Otherwise the defaults (relative to each checkout) suffice.

## 8. Cron jobs

Add to the scraper crontab (see [`app/deploy/meetcal-scrapers.cron`](../app/deploy/meetcal-scrapers.cron)).
Use the venv's python and `PYTHONPATH=<repo>/scrapers`.

```cron
# Meet pipeline: detect new/changed PDFs → scrape → validate → stage → Slack review
0 8 * * *   cd /path/meetcal-backend/scrapers && PYTHONPATH=. usaw/meet_automation/.venv/bin/python -m usaw.meet_automation.pipeline run --all       >> logs/meet-automation.log 2>&1
# On-demand /meet-run requests: drain Slack-triggered runs every couple minutes
*/2 * * * * cd /path/meetcal-backend/scrapers && PYTHONPATH=. usaw/meet_automation/.venv/bin/python -m usaw.meet_automation.pipeline run --requested  >> logs/meet-automation-requests.log 2>&1
# Act on button clicks / replies → Postgres write + confirmation
*/5 * * * * cd /path/meetcal-backend/scrapers && PYTHONPATH=. usaw/meet_automation/.venv/bin/python -m usaw.meet_automation.pipeline approve --all-pending >> logs/meet-automation-approve.log 2>&1
```

The entries job already exists (`run_scraper_job.sh entries`) and now reads
`entries_targets.json` automatically.

## 9. Preview hosting (optional but recommended)

Serve `scrapers/usaw/meet_automation/state/runs/` read-only (static file host /
nginx alias) and set `MEET_AUTOMATION_PREVIEW_BASE_URL` to its public URL. The
Slack review's "Preview the data" link then opens `<base>/<run_id>/preview.html`
from your phone.

## 10. The end-to-end flow

1. `/meet-add 2026-nationals | <meet name> | <usaw page url>` in Slack.
2. The `run` cron sees the page's start-list + schedule PDFs, scrapes + validates,
   posts a Slack review with counts, warnings, a preview link, and **Approve /
   Reject** buttons. No DB writes yet. To trigger this immediately instead of
   waiting for the daily run, use `/meet-run 2026-nationals` (or `/meet-run all`);
   it queues the run for the `run --requested` cron, which stages it within a
   couple minutes and posts the same review. Forces a fresh stage even if the
   PDFs haven't changed.
3. Tap **Approve & publish** (or reply `okay`). The `approve` cron publishes to
   Postgres and posts a confirmation.
4. For entries: `/entries-add <label> | <sport80 entries url>`. The nightly
   entries job scrapes whatever is in the list.

## 11. Operating manually

```bash
PY=usaw/meet_automation/.venv/bin/python; export PYTHONPATH=.
$PY -m usaw.meet_automation.pipeline list                 # staged runs + status
$PY -m usaw.meet_automation.pipeline show <run_id>        # details + preview path
$PY -m usaw.meet_automation.pipeline ingest <run_id>      # publish now
$PY -m usaw.meet_automation.pipeline reject <run_id>      # discard
```

## 12. Tests

```bash
# Rust (API endpoints, signature, routing, store) — needs Postgres:
cd app && cargo test
# Python (validation, discovery, blocks, decision files) — no DB/network:
cd scrapers && PYTHONPATH=. python -m unittest usaw.meet_automation.tests.test_meet_automation
```

## 13. Security notes

- Both endpoints reject any request without a valid Slack signature (v0 HMAC,
  5-minute replay window).
- The API never gets database credentials — approvals are handed to the Python
  pipeline via decision files.
- Restrict who can change config with `MEET_AUTOMATION_SLACK_ALLOWED_USERS`
  and/or the channel allowlist.
