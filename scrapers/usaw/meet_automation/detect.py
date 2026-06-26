"""Detect when a meet page publishes / changes its start-list + schedule PDFs.

Strategy: load the meet page, discover candidate PDF links, classify them as
start-list vs schedule by filename/link-text keywords, then hash the PDF bytes.
A run is "changed" when either PDF URL is new or its content hash differs from
what we last saw (``state/seen.json``). Explicit URLs on the :class:`MeetWatch`
bypass page discovery.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Tuple
from urllib.parse import urljoin

from . import config
from .config import MeetWatch

REQUEST_TIMEOUT_SECONDS = 45
_PDF_HREF_RE = re.compile(r'href\s*=\s*["\']([^"\']+\.pdf[^"\']*)["\']', re.IGNORECASE)
_PDF_BARE_RE = re.compile(r'https?://[^\s"\'<>]+\.pdf[^\s"\'<>]*', re.IGNORECASE)
# Capture a little link text around an anchor so we can classify by label too.
_ANCHOR_RE = re.compile(
    r'<a[^>]*href\s*=\s*["\']([^"\']+\.pdf[^"\']*)["\'][^>]*>(.*?)</a>',
    re.IGNORECASE | re.DOTALL,
)


@dataclass
class DetectResult:
    watch_key: str
    changed: bool
    start_list_url: Optional[str] = None
    schedule_url: Optional[str] = None
    start_list_sha256: Optional[str] = None
    schedule_sha256: Optional[str] = None
    reasons: List[str] = field(default_factory=list)
    candidates: List[str] = field(default_factory=list)


def _verify_arg():
    return os.getenv("REQUESTS_CA_BUNDLE") or os.getenv("CCR_CA_BUNDLE") or True


def fetch_text(url: str) -> str:
    import requests

    resp = requests.get(url, timeout=REQUEST_TIMEOUT_SECONDS, verify=_verify_arg())
    resp.raise_for_status()
    return resp.text


def fetch_bytes(url: str) -> bytes:
    import requests

    resp = requests.get(url, timeout=REQUEST_TIMEOUT_SECONDS, verify=_verify_arg())
    resp.raise_for_status()
    return resp.content


def _classify(url: str, label: str) -> Optional[str]:
    haystack = f"{url} {label}".lower()
    if any(k in haystack for k in config.START_LIST_KEYWORDS):
        return "start_list"
    if any(k in haystack for k in config.SCHEDULE_KEYWORDS):
        return "schedule"
    return None


def discover_pdfs(
    page_html: str, base_url: str = ""
) -> Tuple[Optional[str], Optional[str], List[str]]:
    """Return (start_list_url, schedule_url, all_candidates).

    Relative links are resolved against ``base_url``.
    """
    candidates: List[str] = []
    start_list: Optional[str] = None
    schedule: Optional[str] = None

    def absolute(url: str) -> str:
        return urljoin(base_url, url) if base_url else url

    for url, inner in _ANCHOR_RE.findall(page_html):
        label = re.sub(r"<[^>]+>", " ", inner)
        url = absolute(url)
        if url not in candidates:
            candidates.append(url)
        kind = _classify(url, label)
        if kind == "start_list" and not start_list:
            start_list = url
        elif kind == "schedule" and not schedule:
            schedule = url

    # Fall back to any pdf hrefs / bare links for classification.
    for raw in _PDF_HREF_RE.findall(page_html) + _PDF_BARE_RE.findall(page_html):
        match = absolute(raw)
        if match not in candidates:
            candidates.append(match)
        kind = _classify(match, "")
        if kind == "start_list" and not start_list:
            start_list = match
        elif kind == "schedule" and not schedule:
            schedule = match

    return start_list, schedule, candidates


def _load_seen() -> Dict[str, dict]:
    if config.SEEN_PATH.exists():
        return json.loads(config.SEEN_PATH.read_text(encoding="utf-8"))
    return {}


def _save_seen(seen: Dict[str, dict]) -> None:
    config.SEEN_PATH.parent.mkdir(parents=True, exist_ok=True)
    config.SEEN_PATH.write_text(json.dumps(seen, indent=2), encoding="utf-8")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def detect(watch: MeetWatch, hash_pdfs: bool = True) -> DetectResult:
    """Detect change for one watch. Does not mutate state; call
    :func:`mark_seen` after a successful pipeline run."""
    start_url = watch.start_list_url
    sched_url = watch.schedule_url
    candidates: List[str] = []

    if not (start_url and sched_url):
        page_html = fetch_text(watch.page_url)
        d_start, d_sched, candidates = discover_pdfs(page_html, base_url=watch.page_url)
        start_url = start_url or d_start
        sched_url = sched_url or d_sched

    result = DetectResult(
        watch_key=watch.key,
        changed=False,
        start_list_url=start_url,
        schedule_url=sched_url,
        candidates=candidates,
    )

    if not start_url:
        result.reasons.append("no start-list PDF found on page")
    if not sched_url:
        result.reasons.append("no schedule PDF found on page")

    if hash_pdfs:
        if start_url:
            try:
                result.start_list_sha256 = sha256(fetch_bytes(start_url))
            except Exception as exc:  # noqa: BLE001
                result.reasons.append(f"could not fetch start-list PDF: {exc}")
        if sched_url:
            try:
                result.schedule_sha256 = sha256(fetch_bytes(sched_url))
            except Exception as exc:  # noqa: BLE001
                result.reasons.append(f"could not fetch schedule PDF: {exc}")

    prev = _load_seen().get(watch.key, {})
    for field_name, label in (
        ("start_list_url", "start-list URL"),
        ("schedule_url", "schedule URL"),
        ("start_list_sha256", "start-list content"),
        ("schedule_sha256", "schedule content"),
    ):
        new_val = getattr(result, field_name)
        if new_val and prev.get(field_name) != new_val:
            result.changed = True
            result.reasons.append(
                "new" if field_name not in prev else f"{label} changed"
            )

    if not prev:
        result.reasons.append("never seen before")

    return result


def mark_seen(result: DetectResult) -> None:
    seen = _load_seen()
    seen[result.watch_key] = {
        "start_list_url": result.start_list_url,
        "schedule_url": result.schedule_url,
        "start_list_sha256": result.start_list_sha256,
        "schedule_sha256": result.schedule_sha256,
    }
    _save_seen(seen)
