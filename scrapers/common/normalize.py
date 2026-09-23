"""Value normalisers shared by the Postgres writer and the meet-automation
validator.

This module deliberately has no psycopg dependency so ``usaw.meet_automation``
can import it in environments without a database driver.
"""

from __future__ import annotations

import re
from typing import Any, Optional

# Session platforms the app knows how to render. Anything else is remapped
# client-side (to Red), so an unknown platform is a data error, not a style
# choice. Casing is normalised at ingest so "red" and "RED" never leak through.
KNOWN_PLATFORMS = ("Red", "White", "Blue", "Stars", "Stripes", "Rogue")
_PLATFORM_BY_LOWER = {platform.lower(): platform for platform in KNOWN_PLATFORMS}

# Athletes without a federation membership number (entry lists sometimes omit
# it) carry a deterministic placeholder so re-ingests match the same row.
MEMBER_ID_PLACEHOLDER_PREFIX = "noid:"

# The one spelling of the case- and whitespace-insensitive name rule. It must
# stay identical to `normalize_name` / `normalized_name_sql!` in
# app/src/common/names.rs and the `*_name_normalized` indexes in
# app/migrations/, so the lookup below is index-backed.
NORMALIZED_NAME_SQL = "lower(btrim(regexp_replace(name, '\\s+', ' ', 'g')))"

_TIME_RE = re.compile(
    r"^\s*(?P<hour>\d{1,2})"
    r"(?::(?P<minute>\d{2}))?"
    r"(?::(?P<second>\d{2}))?"
    r"\s*(?:(?P<meridiem>[AaPp])\.?\s*[Mm]\.?)?\s*$"
)


def normalize_name(value: Any) -> str:
    """Collapse internal whitespace, trim, lowercase (names.rs `normalize_name`)."""
    if value is None:
        return ""
    return " ".join(str(value).split()).lower()


def is_placeholder_member_id(value: Any) -> bool:
    """True for a blank member id or one minted by `placeholder_member_id`."""
    if value is None:
        return True
    if not isinstance(value, str):
        return False
    stripped = value.strip()
    return not stripped or stripped.startswith(MEMBER_ID_PLACEHOLDER_PREFIX)


def placeholder_member_id(name: Any) -> str:
    """Deterministic stand-in for a missing member id, derived from the name.

    Mirrors `placeholderMemberId` in usaw/entry_scraper/csv_scraper.js.
    """
    slug = re.sub(r"[^a-z0-9]+", "-", normalize_name(name)).strip("-")
    return f"{MEMBER_ID_PLACEHOLDER_PREFIX}{slug}"


def normalize_platform(value: Any) -> Any:
    """Canonical casing for a session platform ("red" -> "Red").

    Unknown platforms are title-cased so the validator's membership check and
    the stored value agree; None and non-strings pass through untouched.
    """
    if not isinstance(value, str):
        return value
    collapsed = " ".join(value.split())
    if not collapsed:
        return collapsed
    known = _PLATFORM_BY_LOWER.get(collapsed.lower())
    if known:
        return known
    return " ".join(part[:1].upper() + part[1:].lower() for part in collapsed.split())


def parse_time(value: Any) -> Optional[str]:
    """Parse a schedule time into the canonical ``h:mm AM/PM`` form.

    Accepts ``h:mm``, ``HH:MM``, ``h:mm:ss`` (24-hour when no meridiem is
    given) and ``h:mm AM/PM`` / ``h:mm:ss am`` / ``9 AM``. Returns None when
    the value is not a time in one of those shapes.
    """
    if not isinstance(value, str):
        return None
    match = _TIME_RE.match(value)
    if not match:
        return None
    hour = int(match.group("hour"))
    minute = int(match.group("minute") or 0)
    second = int(match.group("second") or 0)
    meridiem = match.group("meridiem")
    if minute > 59 or second > 59:
        return None
    if meridiem:
        if not 1 <= hour <= 12:
            return None
        is_pm = meridiem.lower() == "p"
        hour24 = (hour % 12) + (12 if is_pm else 0)
    else:
        if match.group("minute") is None:
            return None  # a bare "9" is ambiguous without AM/PM
        if hour > 23:
            return None
        hour24 = hour
    suffix = "PM" if hour24 >= 12 else "AM"
    hour12 = hour24 % 12 or 12
    return f"{hour12}:{minute:02d} {suffix}"


def normalize_time(value: Any) -> Any:
    """`parse_time`, but an unparseable value is returned as-is so the row is
    still written (the validator warns about it instead)."""
    if not isinstance(value, str):
        return value
    parsed = parse_time(value)
    return parsed if parsed is not None else value
