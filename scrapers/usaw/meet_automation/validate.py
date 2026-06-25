"""Validity checks for staged start-list + schedule data.

Produces a structured report of findings grouped by code, each tagged
``error`` / ``warning`` / ``info``. Errors mean "almost certainly broken
parse, look before approving"; warnings mean "plausible but worth a glance".
The pipeline never auto-blocks on findings — a human approves in Slack — but
the report drives what that human sees.
"""

from __future__ import annotations

import re
from collections import Counter, defaultdict
from typing import Any, Dict, List, Optional

KNOWN_PLATFORMS = {"Red", "White", "Blue", "Stars", "Stripes", "Rogue"}
ALLOWED_GENDERS = {"Male", "Female", "Men", "Women"}
WEIGHT_CLASS_RE = re.compile(r"^\+?\d{2,3}\+?$")
NAME_FRAGMENT_RE = re.compile(r"\d")
# Competition / overlay tokens that should never leak into a club name. We only
# flag these when they appear ALL-CAPS in the source club string: clubs are
# normally title-cased, so "WSO"/"UNI" stands out as a fused overlay marker,
# whereas a title-cased color word (e.g. "White Rose Barbell") is a real name.
CLUB_LEAK_TOKENS = {"WSO", "UNI", "MIL", "ADAP", "OPEN"}

ERROR = "error"
WARNING = "warning"
INFO = "info"


class _Findings:
    """Aggregates findings by code so the report stays small."""

    def __init__(self) -> None:
        self._by_code: Dict[str, Dict[str, Any]] = {}

    def add(self, severity: str, code: str, message: str, example: Optional[str] = None) -> None:
        bucket = self._by_code.setdefault(
            code,
            {"severity": severity, "code": code, "message": message, "count": 0, "examples": []},
        )
        bucket["count"] += 1
        if example and len(bucket["examples"]) < 5:
            bucket["examples"].append(example)

    def as_list(self) -> List[Dict[str, Any]]:
        order = {ERROR: 0, WARNING: 1, INFO: 2}
        return sorted(
            self._by_code.values(),
            key=lambda f: (order.get(f["severity"], 9), -f["count"]),
        )


def _norm(value: Any) -> str:
    return "" if value is None else str(value).strip()


def validate(
    athletes: List[Dict[str, Any]],
    schedule: List[Dict[str, Any]],
    meet_name: str,
) -> Dict[str, Any]:
    f = _Findings()

    # ---- athlete-level checks ------------------------------------------
    seen_member_ids: Counter = Counter()
    seen_names: Counter = Counter()
    blank_wso = 0
    blank_club = 0

    for a in athletes:
        name = _norm(a.get("name"))
        club = _norm(a.get("club"))
        wso = _norm(a.get("wso"))
        gender = _norm(a.get("gender"))
        weight_class = _norm(a.get("weightClass") or a.get("weight_class"))
        member_id = _norm(a.get("memberId") or a.get("member_id"))
        session_number = a.get("sessionNumber", a.get("session_number"))
        session_platform = _norm(a.get("sessionPlatform") or a.get("session_platform"))

        if member_id:
            seen_member_ids[member_id] += 1
        if name:
            seen_names[name.lower()] += 1

        # names
        if not name:
            f.add(ERROR, "name_empty", "Athlete with empty name", member_id or "?")
        else:
            if len(name.split()) < 2:
                f.add(WARNING, "name_single_token", "Name has only one token", name)
            if NAME_FRAGMENT_RE.search(name):
                f.add(WARNING, "name_has_digit", "Name contains a digit (likely OCR leak)", name)

        # clubs
        if not club or club.lower() == "unaffiliated":
            blank_club += 1
        else:
            if club.startswith("0 ") or "/ / / /" in club:
                f.add(WARNING, "club_suspicious", "Club looks malformed", f"{name} -> {club}")
            leak_tokens = {
                t.strip(",.") for t in club.split()
                if t.isupper() and t.strip(",.") in CLUB_LEAK_TOKENS
            }
            if leak_tokens:
                f.add(
                    WARNING,
                    "club_competition_leak",
                    "Club contains competition/platform text",
                    f"{name} -> {club}",
                )

        # wso
        if not wso:
            blank_wso += 1

        # gender
        if gender and gender not in ALLOWED_GENDERS:
            f.add(ERROR, "gender_invalid", "Unexpected gender value", f"{name} -> {gender}")

        # weight class
        if not weight_class:
            f.add(ERROR, "weight_class_missing", "Missing weight class", name)
        elif not WEIGHT_CLASS_RE.match(weight_class):
            f.add(ERROR, "weight_class_invalid", "Weight class fails pattern", f"{name} -> {weight_class}")

        # session assignment (needed to place the athlete on the schedule)
        if session_number in (None, "", 0) or not session_platform:
            f.add(ERROR, "session_unassigned", "Athlete missing session number/platform", name)
        elif session_platform not in KNOWN_PLATFORMS:
            f.add(WARNING, "platform_unknown", "Unknown session platform", f"{name} -> {session_platform}")

    for member_id, count in seen_member_ids.items():
        if count > 1:
            f.add(WARNING, "member_id_duplicate", "Duplicate memberId", f"{member_id} x{count}")
    for name, count in seen_names.items():
        if count > 1:
            f.add(INFO, "name_duplicate", "Same name appears multiple times", f"{name} x{count}")

    total_athletes = len(athletes)
    if total_athletes == 0:
        f.add(ERROR, "no_athletes", "No athletes parsed", None)
    else:
        if blank_wso == total_athletes:
            f.add(WARNING, "wso_all_blank", "Every athlete is missing a WSO", None)
        if blank_club > 0.5 * total_athletes:
            f.add(
                INFO,
                "club_mostly_blank",
                f"{blank_club}/{total_athletes} athletes have no club",
                None,
            )

    # ---- schedule-level checks -----------------------------------------
    schedule_sessions = set()
    for s in schedule:
        session_id = s.get("sessionId", s.get("session_id"))
        platform = _norm(s.get("platform"))
        start_time = _norm(s.get("startTime") or s.get("start_time"))
        date = _norm(s.get("date"))
        if session_id in (None, "", 0):
            f.add(ERROR, "schedule_session_missing", "Schedule row missing sessionId", platform)
        if not platform:
            f.add(ERROR, "schedule_platform_missing", "Schedule row missing platform", str(session_id))
        elif platform not in KNOWN_PLATFORMS:
            f.add(WARNING, "schedule_platform_unknown", "Unknown schedule platform", platform)
        if not start_time:
            f.add(WARNING, "schedule_no_start_time", "Schedule row missing start time", f"{session_id} {platform}")
        if not date:
            f.add(WARNING, "schedule_no_date", "Schedule row missing date", f"{session_id} {platform}")
        try:
            schedule_sessions.add((int(float(session_id)), platform))
        except (TypeError, ValueError):
            pass

    if len(schedule) == 0:
        f.add(ERROR, "no_schedule", "No schedule rows parsed", None)

    # ---- cross checks: athletes <-> schedule ---------------------------
    athletes_by_session: Dict[Any, int] = defaultdict(int)
    for a in athletes:
        session_number = a.get("sessionNumber", a.get("session_number"))
        session_platform = _norm(a.get("sessionPlatform") or a.get("session_platform"))
        try:
            key = (int(float(session_number)), session_platform)
        except (TypeError, ValueError):
            continue
        athletes_by_session[key] += 1
        if schedule and key not in schedule_sessions:
            f.add(
                ERROR,
                "athlete_session_not_in_schedule",
                "Athlete assigned to a session/platform that has no schedule row",
                f"{a.get('name')} -> session {session_number} {session_platform}",
            )

    for key in schedule_sessions:
        if athletes_by_session.get(key, 0) == 0:
            f.add(
                WARNING,
                "schedule_session_no_athletes",
                "Schedule session has no athletes assigned",
                f"session {key[0]} {key[1]}",
            )

    findings = f.as_list()
    errors = sum(x["count"] for x in findings if x["severity"] == ERROR)
    warnings = sum(x["count"] for x in findings if x["severity"] == WARNING)

    return {
        "meet_name": meet_name,
        "ok": errors == 0,
        "errors": errors,
        "warnings": warnings,
        "counts": {
            "athletes": total_athletes,
            "schedule_rows": len(schedule),
            "sessions": len({k[0] for k in schedule_sessions}),
            "platforms": sorted({k[1] for k in schedule_sessions}),
            "athletes_without_wso": blank_wso,
            "athletes_without_club": blank_club,
        },
        "findings": findings,
    }
