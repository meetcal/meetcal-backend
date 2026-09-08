"""Parser for USAW start-list PDFs with WSO / First Name / Last Name table columns."""

from __future__ import annotations

import re
import sys
from io import BytesIO
from types import ModuleType
from typing import Dict, List, Optional, Sequence, Tuple

import pdfplumber

PLATFORMS = {"RED", "WHITE", "BLUE", "STARS", "STRIPES", "ROGUE"}
ENTRY_TOTAL_MAX = 400
HEADER_FIELD_ALIASES = {
    "wso": "wso",
    "lot": "lot",
    "first name": "first_name",
    "last name": "last_name",
    "nationality": "nationality",
    "year": "year",
    "age": "age",
    "club name": "club",
    "competitions": "competitions",
    "entry": "entry",
    "group": "group",
    "sess.": "session",
    "sess": "session",
    "plat": "platform",
    "day": "day",
    "time": "time",
    "#": "order",
}


def _scraper_module() -> ModuleType:
    for module in sys.modules.values():
        if hasattr(module, "normalize_fragment") and hasattr(
            module, "parse_weight_class_and_gender"
        ):
            return module
    import scraper

    return scraper


def _normalize_fragment(value: str) -> str:
    return _scraper_module().normalize_fragment(value)


def _normalize_name(raw_name: str) -> str:
    return _scraper_module().normalize_name(raw_name)


def _normalize_club(raw_club: str) -> str:
    return _scraper_module().normalize_club(raw_club)


def _normalize_table_row(raw_row: Sequence[Optional[str]]) -> List[str]:
    return _scraper_module().normalize_table_row(raw_row)


def _parse_weight_class_and_gender(competitions: str) -> Tuple[str, str]:
    return _scraper_module().parse_weight_class_and_gender(competitions)


def _header_field_map(header_row: Sequence[str]) -> Dict[str, int]:
    mapping: Dict[str, int] = {}
    for index, header in enumerate(header_row):
        key = HEADER_FIELD_ALIASES.get(header.lower().strip())
        if key:
            mapping[key] = index
    return mapping


def is_wso_table_header(header_row: Sequence[str]) -> bool:
    fields = _header_field_map(header_row)
    required = {"wso", "lot", "first_name", "last_name", "session", "platform", "entry", "competitions"}
    return required.issubset(fields.keys())


def is_wso_table_pdf(pdf_bytes: bytes) -> bool:
    with pdfplumber.open(BytesIO(pdf_bytes)) as pdf:
        if not pdf.pages:
            return False
        for table in pdf.pages[0].extract_tables() or []:
            if table and is_wso_table_header(_normalize_table_row(table[0])):
                return True
    return False


CLUB_BLEED_SUFFIXES = (
    re.compile(r"\s+OPEN\s+[MW]\s+\d{2,3}\+?$", re.IGNORECASE),
    re.compile(r"\s+OPEN\s+[MW]$", re.IGNORECASE),
    re.compile(r"\s+OPEN$", re.IGNORECASE),
    re.compile(r"\s+OPE$", re.IGNORECASE),
    re.compile(r"\s+OP$", re.IGNORECASE),
    re.compile(r"\s+N$", re.IGNORECASE),
)


def _strip_club_competition_bleed(club: str) -> str:
    cleaned = club.strip()
    while True:
        previous = cleaned
        for pattern in CLUB_BLEED_SUFFIXES:
            cleaned = pattern.sub("", cleaned).strip()
        if cleaned == previous:
            break
    return cleaned


def _clean_club(raw_club: str) -> str:
    club = _strip_club_competition_bleed(_normalize_fragment(raw_club))
    return _normalize_club(club)


OPEN_WEIGHT_PATTERNS: Tuple[Tuple[re.Pattern[str], str], ...] = (
    (re.compile(r"OPEN\s+W\s+(\d{2,3}\+?)", re.IGNORECASE), "Female"),
    (re.compile(r"OPEN\s+M\s+(\d{2,3}\+?)", re.IGNORECASE), "Male"),
    (re.compile(r"(?:JR|JUNIOR)\s+W\s+(\d{2,3}\+?)", re.IGNORECASE), "Female"),
    (re.compile(r"(?:JR|JUNIOR)\s+M\s+(\d{2,3}\+?)", re.IGNORECASE), "Male"),
    (re.compile(r"ADAP\s+W\s+(\d{2,3}\+?)", re.IGNORECASE), "Female"),
    (re.compile(r"ADAP\s+M\s+(\d{2,3}\+?)", re.IGNORECASE), "Male"),
    (re.compile(r"UNI\s+W\s+(\d{2,3}\+?)", re.IGNORECASE), "Female"),
    (re.compile(r"UNI\s+M\s+(\d{2,3}\+?)", re.IGNORECASE), "Male"),
    (re.compile(r"U\d{1,2}\s+W\s+(\d{2,3}\+?)", re.IGNORECASE), "Female"),
    (re.compile(r"U\d{1,2}\s+M\s+(\d{2,3}\+?)", re.IGNORECASE), "Male"),
    (re.compile(r"14-15YO\s+W\s+(\d{2,3}\+?)", re.IGNORECASE), "Female"),
    (re.compile(r"14-15YO\s+M\s+(\d{2,3}\+?)", re.IGNORECASE), "Male"),
    (re.compile(r"16-17YO\s+W\s+(\d{2,3}\+?)", re.IGNORECASE), "Female"),
    (re.compile(r"16-17YO\s+M\s+(\d{2,3}\+?)", re.IGNORECASE), "Male"),
)
FUSED_110_ENTRY_PATTERN = re.compile(
    r"(?:\d)?112(\d{2})\+?(\d)?/?(\d)?"
)
FUSED_111_ENTRY_PATTERN = re.compile(
    r"(?:\d)?111(\d{2})\+?(\d)?/?(\d)?"
)
WEIGHT_CLASS_BLEED_ENTRY_PATTERN = re.compile(r"861\+(\d)\s*/(\d)")
FUSED_110_SLASH_ENTRY_PATTERN = re.compile(r"1101\s+(\d)/(\d)")
FUSED_112_SLASH_ENTRY_PATTERN = re.compile(r"1102\s+(\d)/(\d)")
MIL_WSO_ENTRY_PATTERN = re.compile(r"MIL\s*(\d)([MW])(\d*)\s*(\d*)", re.IGNORECASE)
M1IL_ENTRY_PATTERN = re.compile(r"M1IL(\d)\s*W(\d)", re.IGNORECASE)
MI2L0_ENTRY_PATTERN = re.compile(r"MI2L0\s*M?0", re.IGNORECASE)
M26IL_ENTRY_PATTERN = re.compile(r"M26IL(\d)", re.IGNORECASE)
ENTRY_TOTAL_MIN = 15


def _repair_competitions(raw: str) -> str:
    text = _normalize_fragment(raw)
    text = re.sub(r"^NW\s+", "OPEN W ", text, flags=re.IGNORECASE)
    text = re.sub(r"^NE\s+", "OPEN W ", text, flags=re.IGNORECASE)
    text = re.sub(r"^N\s+W\s+", "OPEN W ", text, flags=re.IGNORECASE)
    text = re.sub(r"^EN\s+M\s+", "OPEN M ", text, flags=re.IGNORECASE)
    text = re.sub(r"^EN\s+W\s+", "OPEN W ", text, flags=re.IGNORECASE)
    return text


def _normalize_weight_class(weight: str) -> str:
    cleaned = weight.strip()
    if cleaned.startswith("+") and cleaned[1:].isdigit():
        return f"{cleaned[1:]}+"
    if cleaned.endswith("+") and cleaned[:-1].isdigit():
        return cleaned
    return cleaned


def _parse_wso_table_weight_class_and_gender(competitions: str) -> Tuple[str, str]:
    text = _repair_competitions(competitions)
    for pattern, gender in OPEN_WEIGHT_PATTERNS:
        match = pattern.search(text)
        if match:
            return _normalize_weight_class(match.group(1)), gender

    for gender_token, gender in (("W", "Female"), ("M", "Male")):
        match = re.search(
            rf"\b{gender_token}\s+(\d{{2,3}}\+?)\b",
            text,
            flags=re.IGNORECASE,
        )
        if match:
            return _normalize_weight_class(match.group(1)), gender

    return _parse_weight_class_and_gender(text)


def _interpret_fused_111_entry(core: str, suffix: str = "") -> Optional[int]:
    if len(core) != 2 or not core.isdigit():
        return None
    base = 100 + int(core[::-1])
    if suffix and suffix.isdigit():
        base += int(suffix)
    if 100 <= base <= ENTRY_TOTAL_MAX:
        return base
    return None


def _interpret_fused_110_entry(
    core: str, suffix: str = "", trailing: str = ""
) -> Optional[int]:
    if len(core) != 2 or not core.isdigit():
        return None
    base = 200 + int(core[::-1])
    if suffix and suffix.isdigit():
        base += int(suffix)
    if 100 <= base <= ENTRY_TOTAL_MAX:
        return base
    return None


def _plausible_entry_total(value: int) -> Optional[int]:
    if ENTRY_TOTAL_MIN <= value <= ENTRY_TOTAL_MAX:
        return value
    return None


def _parse_mil_wso_entry(raw: str) -> Optional[int]:
    match = MIL_WSO_ENTRY_PATTERN.search(raw)
    if not match:
        return None

    prefix, _gender, middle, trailing = match.groups()
    prefix_value = int(prefix)

    if prefix_value == 2 and middle == "0" and trailing:
        return _plausible_entry_total(prefix_value * 100 - 10)

    if len(middle) == 2 and middle.startswith("0"):
        return _plausible_entry_total(100 + int(middle))

    if len(middle) == 1 and len(trailing) >= 2:
        return _plausible_entry_total(100 + int(middle) * 10 + int(trailing[0]))

    if len(middle) == 2 and not middle.startswith("0") and len(trailing) == 1:
        return _plausible_entry_total(100 + int(middle))

    return None


def _parse_entry_total(
    raw_entry: str,
    group: str = "",
    competitions: str = "",
) -> Optional[int]:
    raw = _normalize_fragment(raw_entry)
    group = _normalize_fragment(group)
    entry_blob = f"{raw} {group}"

    if re.search(r"/\s*1\s+00\b", raw):
        return 100

    split_match = re.search(r"/\s*(\d)\s+(\d{2})\b", raw)
    if split_match:
        combined = int(split_match.group(1) + split_match.group(2))
        plausible = _plausible_entry_total(combined)
        if plausible is not None:
            return plausible

    weight_bleed_match = WEIGHT_CLASS_BLEED_ENTRY_PATTERN.search(raw)
    if weight_bleed_match:
        rebuilt = 100 + int(weight_bleed_match.group(1)) * 10 + int(
            weight_bleed_match.group(2)
        )
        plausible = _plausible_entry_total(rebuilt)
        if plausible is not None:
            return plausible

    slash_match = FUSED_110_SLASH_ENTRY_PATTERN.search(raw)
    if slash_match:
        rebuilt = 100 + int(slash_match.group(1)) * 10 + int(slash_match.group(2))
        plausible = _plausible_entry_total(rebuilt)
        if plausible is not None:
            return plausible

    slash_match = FUSED_112_SLASH_ENTRY_PATTERN.search(raw)
    if slash_match:
        rebuilt = 200 + int(slash_match.group(1)) * 10 + int(slash_match.group(2))
        plausible = _plausible_entry_total(rebuilt)
        if plausible is not None:
            return plausible

    mil_wso_total = _parse_mil_wso_entry(raw)
    if mil_wso_total is not None:
        return mil_wso_total

    m1il_match = M1IL_ENTRY_PATTERN.search(raw)
    if m1il_match:
        rebuilt = 100 + int(m1il_match.group(1)) * 10 + int(m1il_match.group(2))
        plausible = _plausible_entry_total(rebuilt)
        if plausible is not None:
            return plausible

    if MI2L0_ENTRY_PATTERN.search(raw):
        return 200

    m26il_match = M26IL_ENTRY_PATTERN.search(entry_blob)
    if m26il_match:
        rebuilt = 260 + int(m26il_match.group(1))
        plausible = _plausible_entry_total(rebuilt)
        if plausible is not None:
            return plausible

    compact = re.sub(r"\s+", "", raw)
    fused_match = FUSED_110_ENTRY_PATTERN.search(compact)
    if fused_match:
        rebuilt = _interpret_fused_110_entry(
            fused_match.group(1),
            fused_match.group(2) or "",
            fused_match.group(3) or "",
        )
        if rebuilt is not None:
            return rebuilt

    fused_match = FUSED_111_ENTRY_PATTERN.search(compact)
    if fused_match:
        rebuilt = _interpret_fused_111_entry(
            fused_match.group(1),
            fused_match.group(2) or "",
        )
        if rebuilt is not None:
            return rebuilt

    numbers = [int(value) for value in re.findall(r"\d+", raw)]
    plausible = [value for value in numbers if ENTRY_TOTAL_MIN <= value <= ENTRY_TOTAL_MAX]
    if plausible:
        return plausible[-1]

    return None


def _cell(row: Sequence[str], index: Optional[int]) -> str:
    if index is None or index >= len(row):
        return ""
    return str(row[index] or "").strip()


def _parse_platform(raw_platform: str) -> str:
    normalized = _normalize_fragment(raw_platform).upper()
    if normalized not in PLATFORMS:
        return ""
    return normalized.title()


def _parse_row(
    row: Sequence[str],
    field_map: Dict[str, int],
    meet_name: str,
) -> Optional[Dict[str, object]]:
    lot = _cell(row, field_map.get("lot"))
    first_name = _cell(row, field_map.get("first_name"))
    last_name = _cell(row, field_map.get("last_name"))
    age_text = _cell(row, field_map.get("age"))
    session_text = _cell(row, field_map.get("session"))
    platform = _parse_platform(_cell(row, field_map.get("platform")))
    competitions = _repair_competitions(_cell(row, field_map.get("competitions")))
    group = _cell(row, field_map.get("group"))
    entry_total = _parse_entry_total(
        _cell(row, field_map.get("entry")),
        group=group,
        competitions=competitions,
    )

    if not lot.isdigit() or not session_text.isdigit() or not platform:
        return None
    if not first_name or not last_name or not age_text.isdigit():
        return None
    if entry_total is None or entry_total <= 0:
        return None

    age = int(age_text)
    if age < 5 or age > 100:
        return None

    name = _normalize_name(f"{first_name} {last_name}")
    if len(name.split()) < 2:
        return None

    weight_class, gender = _parse_wso_table_weight_class_and_gender(competitions)
    if not weight_class or not gender:
        return None

    wso = _normalize_fragment(_cell(row, field_map.get("wso")))
    club = _clean_club(_cell(row, field_map.get("club")))

    return {
        "adaptive": "ADAP" in competitions.upper(),
        "age": age,
        "club": club,
        "entryTotal": entry_total,
        "gender": gender,
        "meet": meet_name,
        "memberId": lot,
        "name": name,
        "sessionNumber": int(session_text),
        "sessionPlatform": platform,
        "weightClass": weight_class,
        "wso": wso,
    }


def count_wso_table_rows(pdf_bytes: bytes) -> int:
    field_map: Optional[Dict[str, int]] = None
    count = 0

    with pdfplumber.open(BytesIO(pdf_bytes)) as pdf:
        for page in pdf.pages:
            for table in page.extract_tables() or []:
                if not table:
                    continue
                normalized_rows = [_normalize_table_row(raw_row) for raw_row in table]
                start_index = 0
                if is_wso_table_header(normalized_rows[0]):
                    field_map = _header_field_map(normalized_rows[0])
                    start_index = 1
                if field_map is None:
                    continue
                for row in normalized_rows[start_index:]:
                    if _parse_row(row, field_map, meet_name=""):
                        count += 1
    return count


def extract_wso_table_entries(pdf_bytes: bytes, meet_name: str) -> List[Dict[str, object]]:
    entries: List[Dict[str, object]] = []
    field_map: Optional[Dict[str, int]] = None

    with pdfplumber.open(BytesIO(pdf_bytes)) as pdf:
        for page in pdf.pages:
            for table in page.extract_tables() or []:
                if not table:
                    continue
                normalized_rows = [_normalize_table_row(raw_row) for raw_row in table]
                start_index = 0
                if is_wso_table_header(normalized_rows[0]):
                    field_map = _header_field_map(normalized_rows[0])
                    start_index = 1
                if field_map is None:
                    continue
                for row in normalized_rows[start_index:]:
                    parsed = _parse_row(row, field_map, meet_name)
                    if parsed:
                        entries.append(parsed)

    return entries
