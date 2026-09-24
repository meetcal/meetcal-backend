"""Slack review notification + reply-based approval.

Two transports:

* ``chat.postMessage`` (bot token + channel) — preferred, because it returns a
  message ``ts`` we can later poll for your reply. This is what enables the
  "reply ``okay`` in the thread to publish" flow.
* Incoming webhook — notification only (no ``ts``); approval then falls back to
  the ``pipeline.py approve <run_id>`` CLI.

``poll_approval`` reads the message's thread replies and returns ``approved`` /
``rejected`` based on the first decisive word from a non-bot user.
"""

from __future__ import annotations

import re
from typing import Any, Dict, Iterable, List, Optional

from .config import SlackConfig
from .models import StagedBundle, SlackRef

_API = "https://slack.com/api"
REQUEST_TIMEOUT_SECONDS = 30
# Slack block limits and review-message budget, declared rather than inlined.
MAX_HEADER_CHARS = 150  # Slack rejects a header block longer than this
MAX_SECTION_CHARS = 2900  # mrkdwn section limit is 3000; leave headroom
MAX_FINDINGS_SHOWN = 8  # the rest stay in the HTML preview
# Ceiling on thread replies scanned for an approve/reject word. Also sent to
# Slack as `limit`, so the response itself is bounded rather than trusted.
MAX_THREAD_REPLIES = 200
# Reject words that are also ordinary negations inside an approval ("no
# issues, ship it"). They only count as a rejection when they open the reply.
LEADING_ONLY_REJECT_WORDS = frozenset({"no"})
_WORD_SPLIT_RE = re.compile(r"[^\w']+")


def classify_reply(
    text: str, approve_words: Iterable[str], reject_words: Iterable[str]
) -> Optional[str]:
    """Decide 'approved' / 'rejected' / None for one thread reply.

    1. An explicit reject word anywhere ("reject", "stop", "cancel", "don't")
       rejects, whatever else the message says: the safe failure mode is to
       not publish.
    2. Otherwise an approve word anywhere approves, so "no issues, ship it"
       is an approval.
    3. Otherwise a leading-only reject word that opens the reply ("no",
       "no, redo it") rejects.
    """
    words = [w for w in _WORD_SPLIT_RE.split((text or "").strip().lower()) if w]
    if not words:
        return None
    approve = {w.lower() for w in approve_words}
    reject = {w.lower() for w in reject_words}
    bag = set(words)
    if bag & (reject - LEADING_ONLY_REJECT_WORDS):
        return "rejected"
    if bag & approve:
        return "approved"
    if words[0] in reject:
        return "rejected"
    return None


def _preview_link(cfg: SlackConfig, bundle: StagedBundle) -> Optional[str]:
    if cfg.preview_base_url:
        return f"{cfg.preview_base_url.rstrip('/')}/{bundle.run_id}/preview.html"
    return None


def build_blocks(cfg: SlackConfig, bundle: StagedBundle) -> List[Dict[str, Any]]:
    v = bundle.validation or {}
    counts = v.get("counts", {})
    status_emoji = ":white_check_mark:" if v.get("ok") else ":warning:"
    header = f"{status_emoji} {bundle.meet_name}"[:MAX_HEADER_CHARS]

    summary = (
        f"*{counts.get('athletes', 0)}* athletes · "
        f"*{counts.get('schedule_rows', 0)}* schedule rows · "
        f"*{counts.get('sessions', 0)}* sessions\n"
        f"errors: *{v.get('errors', 0)}* · warnings: *{v.get('warnings', 0)}* · "
        f"no WSO: {counts.get('athletes_without_wso', 0)} · "
        f"no club: {counts.get('athletes_without_club', 0)}"
    )

    top = []
    for f in v.get("findings", [])[:MAX_FINDINGS_SHOWN]:
        mark = {"error": ":red_circle:", "warning": ":large_yellow_circle:"}.get(
            f["severity"], ":white_circle:"
        )
        ex = f" — _{f['examples'][0]}_" if f.get("examples") else ""
        top.append(f"{mark} {f['message']} (x{f['count']}){ex}")
    findings_text = "\n".join(top) if top else ":tada: No validation issues."

    blocks: List[Dict[str, Any]] = [
        {"type": "header", "text": {"type": "plain_text", "text": header}},
        {"type": "section", "text": {"type": "mrkdwn", "text": summary}},
        {
            "type": "section",
            "text": {"type": "mrkdwn", "text": findings_text[:MAX_SECTION_CHARS]},
        },
    ]

    links = []
    preview = _preview_link(cfg, bundle)
    if preview:
        links.append(f"<{preview}|📋 Preview the data>")
    if bundle.source.start_list_url:
        links.append(f"<{bundle.source.start_list_url}|start-list PDF>")
    if bundle.source.schedule_url:
        links.append(f"<{bundle.source.schedule_url}|schedule PDF>")
    if links:
        blocks.append({"type": "section", "text": {"type": "mrkdwn", "text": " · ".join(links)}})

    # Interactive buttons. The Rust API's /scrapers/slack/interactions endpoint
    # receives the click and records the decision for the approve cron. The
    # button value carries the run id.
    blocks.append(
        {
            "type": "actions",
            "block_id": "meet_approval",
            "elements": [
                {
                    "type": "button",
                    "action_id": "meet_approve",
                    "style": "primary",
                    "text": {"type": "plain_text", "text": "✅ Approve & publish"},
                    "value": bundle.run_id,
                    "confirm": {
                        "title": {"type": "plain_text", "text": "Publish to Postgres?"},
                        "text": {"type": "mrkdwn", "text": f"Publish `{bundle.run_id}`?"},
                        "confirm": {"type": "plain_text", "text": "Publish"},
                        "deny": {"type": "plain_text", "text": "Cancel"},
                    },
                },
                {
                    "type": "button",
                    "action_id": "meet_reject",
                    "style": "danger",
                    "text": {"type": "plain_text", "text": "🗑 Reject"},
                    "value": bundle.run_id,
                },
            ],
        }
    )
    blocks.append(
        {
            "type": "context",
            "elements": [
                {
                    "type": "mrkdwn",
                    "text": (
                        f"Run `{bundle.run_id}`. Use the buttons, or reply *okay* / *reject* "
                        f"in this thread."
                    ),
                }
            ],
        }
    )
    return blocks


def post_review(cfg: SlackConfig, bundle: StagedBundle) -> SlackRef:
    import requests

    blocks = build_blocks(cfg, bundle)
    fallback = f"{bundle.meet_name}: {(bundle.validation or {}).get('counts', {}).get('athletes', 0)} athletes staged for review"

    if cfg.bot_token and cfg.channel:
        resp = requests.post(
            f"{_API}/chat.postMessage",
            headers={"Authorization": f"Bearer {cfg.bot_token}"},
            json={"channel": cfg.channel, "text": fallback, "blocks": blocks},
            timeout=REQUEST_TIMEOUT_SECONDS,
        )
        data = resp.json()
        if not data.get("ok"):
            raise RuntimeError(f"slack chat.postMessage failed: {data.get('error')}")
        return SlackRef(channel=data.get("channel"), ts=data.get("ts"))

    if cfg.webhook_url:
        resp = requests.post(
            cfg.webhook_url,
            json={"text": fallback, "blocks": blocks},
            timeout=REQUEST_TIMEOUT_SECONDS,
        )
        resp.raise_for_status()
        return SlackRef(channel=None, ts=None)

    raise RuntimeError(
        "Slack not configured: set SLACK_BOT_TOKEN + SLACK_MEET_AUTOMATION_CHANNEL "
        "(for reply approval) or SLACK_MEET_AUTOMATION_WEBHOOK_URL (notify only)."
    )


def poll_approval(cfg: SlackConfig, bundle: StagedBundle) -> Optional[str]:
    """Return 'approved' / 'rejected' / None by reading the thread replies.

    Requires a bot token + the channel/ts captured when the review was posted.
    """
    import requests

    if not (cfg.bot_token and bundle.slack.channel and bundle.slack.ts):
        return None

    resp = requests.get(
        f"{_API}/conversations.replies",
        headers={"Authorization": f"Bearer {cfg.bot_token}"},
        params={
            "channel": bundle.slack.channel,
            "ts": bundle.slack.ts,
            "limit": MAX_THREAD_REPLIES,
        },
        timeout=REQUEST_TIMEOUT_SECONDS,
    )
    data = resp.json()
    if not data.get("ok"):
        raise RuntimeError(f"slack conversations.replies failed: {data.get('error')}")

    for msg in list(data.get("messages", []))[:MAX_THREAD_REPLIES]:
        if msg.get("ts") == bundle.slack.ts:
            continue  # the root review message itself
        if msg.get("bot_id"):
            continue  # ignore the bot's own posts
        # Same allowlist the button endpoint enforces: when set, only these users
        # can approve/reject by reply.
        if cfg.allowed_users and msg.get("user") not in cfg.allowed_users:
            continue
        decision = classify_reply(msg.get("text") or "", cfg.approve_words, cfg.reject_words)
        if decision is not None:
            return decision
    return None


def post_thread_reply(cfg: SlackConfig, bundle: StagedBundle, text: str) -> None:
    import requests

    if not (cfg.bot_token and bundle.slack.channel and bundle.slack.ts):
        if cfg.webhook_url:
            requests.post(cfg.webhook_url, json={"text": text}, timeout=REQUEST_TIMEOUT_SECONDS)
        return
    requests.post(
        f"{_API}/chat.postMessage",
        headers={"Authorization": f"Bearer {cfg.bot_token}"},
        json={"channel": bundle.slack.channel, "thread_ts": bundle.slack.ts, "text": text},
        timeout=REQUEST_TIMEOUT_SECONDS,
    )
