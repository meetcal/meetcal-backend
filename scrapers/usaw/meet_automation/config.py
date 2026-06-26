"""Configuration for the meet automation pipeline.

A pipeline run is driven by a :class:`MeetWatch`: which page to watch, the
canonical meet name to stamp on every row, and how to find the start-list and
schedule PDFs on that page. Watches can be declared in ``watches.json`` next to
this file (see ``watches.example.json``) or passed on the command line.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional

MODULE_DIR = Path(__file__).resolve().parent
STATE_DIR = Path(os.getenv("MEET_AUTOMATION_STATE_DIR", MODULE_DIR / "state"))
RUNS_DIR = STATE_DIR / "runs"
SEEN_PATH = STATE_DIR / "seen.json"

# Shared with the Rust API's /meet-* commands. Both sides must agree on this
# path so Slack-managed watches are visible to the pipeline (especially in a
# separate-checkout setup where it points at a shared location).
WATCHES_PATH = Path(os.getenv("MEET_AUTOMATION_WATCHES_PATH", MODULE_DIR / "watches.json"))

# Repo root: scrapers/usaw/meet_automation -> repo root is 3 parents up.
REPO_ROOT = MODULE_DIR.parents[2]
SCRAPERS_DIR = REPO_ROOT / "scrapers"

FINAL_START_SCRAPER = (
    SCRAPERS_DIR / "usaw" / "final_start_scraper" / "scraper.py"
)
SCHEDULE_SCRAPER = (
    SCRAPERS_DIR / "usaw" / "owlcms_schedule_scraper" / "final_scraper.py"
)

# Filename keywords used to classify PDF links discovered on a meet page.
START_LIST_KEYWORDS = ("start list", "start_list", "startlist", "start-list")
SCHEDULE_KEYWORDS = ("schedule",)


@dataclass
class MeetWatch:
    """One meet to watch and (eventually) ingest."""

    key: str
    """Stable slug used for state files and run ids, e.g. ``2026-nationals``."""

    meet_name: str
    """Canonical meet name stamped on every athlete/schedule/meet row.

    Must match the ``meets.name`` already synced by the meet-sync job so the
    mobile API joins line up.
    """

    page_url: str
    """USAW meet page to scan for PDF links."""

    start_list_url: Optional[str] = None
    """Explicit start-list PDF URL. Overrides page discovery when set."""

    schedule_url: Optional[str] = None
    """Explicit schedule PDF URL. Overrides page discovery when set."""

    source_format: str = "auto"
    """Start-list parser format hint: auto|masters|owlcms|registration."""

    start_member_id: int = 3100
    """First synthetic member id for athletes without one."""

    schedule_start_id: int = 1
    """First id assigned to schedule rows."""

    default_year: int = 2026
    """Year used for schedule dates that lack one."""

    def __post_init__(self) -> None:
        if not self.key or not self.meet_name or not self.page_url:
            raise ValueError("MeetWatch requires key, meet_name and page_url")

    @classmethod
    def from_dict(cls, data: dict) -> "MeetWatch":
        known = {f.name for f in cls.__dataclass_fields__.values()}  # type: ignore[attr-defined]
        return cls(**{k: v for k, v in data.items() if k in known})

    def to_dict(self) -> dict:
        return {
            "key": self.key,
            "meet_name": self.meet_name,
            "page_url": self.page_url,
            "start_list_url": self.start_list_url,
            "schedule_url": self.schedule_url,
            "source_format": self.source_format,
            "start_member_id": self.start_member_id,
            "schedule_start_id": self.schedule_start_id,
            "default_year": self.default_year,
        }


@dataclass
class SlackConfig:
    """Slack wiring. Webhook is enough to notify; bot token + channel are
    required for automated reply-based approval polling."""

    webhook_url: Optional[str] = None
    bot_token: Optional[str] = None
    channel: Optional[str] = None
    approve_words: List[str] = field(
        default_factory=lambda: ["okay", "ok", "approve", "approved", "yes", "ship", "lgtm"]
    )
    reject_words: List[str] = field(
        default_factory=lambda: ["reject", "no", "nope", "stop", "cancel"]
    )
    preview_base_url: Optional[str] = None
    """If set, preview links become ``{preview_base_url}/{run_id}/preview.html``.
    Otherwise the Slack message references the local preview path only."""

    allowed_users: List[str] = field(default_factory=list)
    """Slack user ids permitted to approve. Empty = anyone in the channel. Mirrors
    the Rust button endpoint's MEET_AUTOMATION_SLACK_ALLOWED_USERS allowlist so
    reply-based approval enforces the same restriction."""

    @classmethod
    def from_env(cls) -> "SlackConfig":
        allowed = os.getenv("MEET_AUTOMATION_SLACK_ALLOWED_USERS", "")
        return cls(
            webhook_url=os.getenv("SLACK_MEET_AUTOMATION_WEBHOOK_URL")
            or os.getenv("SLACK_MEET_WEBHOOK_URL"),
            bot_token=os.getenv("SLACK_BOT_TOKEN"),
            channel=os.getenv("SLACK_MEET_AUTOMATION_CHANNEL"),
            preview_base_url=os.getenv("MEET_AUTOMATION_PREVIEW_BASE_URL"),
            allowed_users=[u for u in allowed.replace(",", " ").split() if u],
        )


def load_watches(path: Optional[Path] = None) -> List[MeetWatch]:
    path = path or WATCHES_PATH
    if not path.exists():
        return []
    data = json.loads(path.read_text(encoding="utf-8"))
    return [MeetWatch.from_dict(entry) for entry in data]


def find_watch(key: str, path: Optional[Path] = None) -> Optional[MeetWatch]:
    for watch in load_watches(path):
        if watch.key == key:
            return watch
    return None
