"""Drive the existing USAW scrapers to extract staged rows.

This reuses the already-maintained parsers in
``final_start_scraper/scraper.py`` and
``owlcms_schedule_scraper/final_scraper.py`` by importing them from their file
paths. It calls only their pure extraction functions — it never runs their
``main``/ingest paths and never modifies them — so all the hard-won PDF parsing
logic stays in one place.
"""

from __future__ import annotations

import importlib.util
import sys
from io import BytesIO
from types import ModuleType
from typing import Any, Dict, List, Optional, Tuple

from . import config
from .config import MeetWatch

_MODULE_CACHE: Dict[str, ModuleType] = {}


def _load_module(path, alias: str) -> ModuleType:
    if alias in _MODULE_CACHE:
        return _MODULE_CACHE[alias]
    # Ensure scrapers dir is importable for any sibling imports.
    scrapers_dir = str(config.SCRAPERS_DIR)
    if scrapers_dir not in sys.path:
        sys.path.insert(0, scrapers_dir)
    spec = importlib.util.spec_from_file_location(alias, str(path))
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load scraper module from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[alias] = module
    spec.loader.exec_module(module)
    _MODULE_CACHE[alias] = module
    return module


def scrape_start_list(
    pdf_bytes: bytes,
    meet_name: str,
    source_format: str = "auto",
    start_member_id: int = 3100,
) -> List[Dict[str, Any]]:
    mod = _load_module(config.FINAL_START_SCRAPER, "meet_automation_start_scraper")
    detected = mod.detect_source_format(pdf_bytes, source_format)
    entries = mod.extract_entries(
        BytesIO(pdf_bytes),
        meet_name=meet_name,
        source_format=detected,
        start_member_id=start_member_id,
    )
    # entries are already in Convex shape (camelCase dicts).
    return [dict(e) for e in entries]


def scrape_schedule(
    pdf_bytes: bytes,
    meet_name: str,
    start_list_url: Optional[str] = None,
    start_id: int = 1,
) -> List[Dict[str, Any]]:
    mod = _load_module(config.SCHEDULE_SCRAPER, "meet_automation_schedule_scraper")
    rows = mod.extract_schedule_data(
        pdf_file=BytesIO(pdf_bytes),
        meet_name=meet_name,
        start_list_url=start_list_url,
    )
    rows_with_ids = mod.add_ids(rows, start_id=start_id)
    # to_ts_object() is the clean camelCase dict (no scraperSecret).
    return [row.to_ts_object() for row in rows_with_ids]


def scrape(
    watch: MeetWatch,
    start_list_bytes: bytes,
    schedule_bytes: bytes,
    start_list_url: Optional[str] = None,
) -> Tuple[List[Dict[str, Any]], List[Dict[str, Any]], Dict[str, Any]]:
    """Return (athletes, schedule, stats).

    ``start_list_url`` is the resolved start-list PDF URL (from auto-discovery);
    the schedule scraper uses it to fill weight-class labels for schedules that
    don't carry classes directly. Falls back to the watch's explicit override.
    """
    athletes = scrape_start_list(
        start_list_bytes,
        meet_name=watch.meet_name,
        source_format=watch.source_format,
        start_member_id=watch.start_member_id,
    )
    schedule = scrape_schedule(
        schedule_bytes,
        meet_name=watch.meet_name,
        start_list_url=start_list_url or watch.start_list_url,
        start_id=watch.schedule_start_id,
    )
    stats = {
        "athletes_parsed": len(athletes),
        "schedule_rows_parsed": len(schedule),
        "start_list_bytes": len(start_list_bytes),
        "schedule_bytes": len(schedule_bytes),
    }
    return athletes, schedule, stats
