from __future__ import annotations

import hashlib
import logging
import os
import time
from contextlib import contextmanager
from decimal import Decimal
from typing import Any, Iterable

import psycopg
from psycopg.rows import dict_row

from common.normalize import (
    NORMALIZED_NAME_SQL,
    is_placeholder_member_id,
    normalize_name,
    normalize_platform,
    normalize_time,
)

logger = logging.getLogger(__name__)


def database_url() -> str:
    url = os.getenv("DATABASE_URL")
    if not url:
        raise RuntimeError("DATABASE_URL must be set for Postgres scraper writes")
    return url


@contextmanager
def connect():
    with psycopg.connect(database_url(), row_factory=dict_row) as conn:
        yield conn


def stable_id(prefix: str, *parts: Any) -> str:
    raw = "|".join("" if part is None else str(part) for part in parts)
    digest = hashlib.sha1(raw.encode("utf-8")).hexdigest()
    return f"{prefix}_{digest}"


def millis() -> int:
    return int(time.time() * 1000)


def first(row: dict[str, Any], *keys: str, default: Any = None) -> Any:
    for key in keys:
        if key in row and row[key] is not None:
            return row[key]
    return default


def clean(row: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in row.items() if key != "scraperSecret"}


def title_case_value(value: Any) -> Any:
    if not isinstance(value, str):
        return value
    stripped = value.strip()
    if not stripped:
        return stripped
    return " ".join(part[:1].upper() + part[1:].lower() for part in stripped.split())


def normalize_gender(value: Any) -> Any:
    if not isinstance(value, str):
        return value
    normalized = value.strip()
    lookup = {
        "men": "Men",
        "women": "Women",
        "male": "Male",
        "female": "Female",
        "m": "Men",
        "f": "Women",
    }
    return lookup.get(normalized.lower(), title_case_value(normalized))


def normalize_age_category(value: Any) -> Any:
    if not isinstance(value, str):
        return value
    normalized = title_case_value(value)
    age_codes = {
        "u13": "U13",
        "u15": "U15",
        "u17": "U17",
        "u20": "U20",
        "u25": "U25",
    }
    return age_codes.get(normalized.lower(), normalized)


def normalize_federation(value: Any) -> Any:
    if not isinstance(value, str):
        return value
    normalized = value.strip()
    if normalized.lower() in {"usaw", "iwf", "umwf", "bwl"}:
        return normalized.upper()
    return normalized


def comparable(value: Any) -> Any:
    if isinstance(value, Decimal):
        return float(value)
    return value


def row_changed(existing: dict[str, Any] | None, values: dict[str, Any]) -> bool:
    if existing is None:
        return True
    return any(comparable(existing.get(key)) != comparable(value) for key, value in values.items())


def require_text(value: Any, field: str) -> str:
    """Guard for identity fields a destructive write keys off. Empty or
    non-string values raise rather than widening the statement's blast radius."""
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{field} is required")
    return value


def delete_athletes_by_meet(conn, meet: Any) -> int:
    """Delete every athlete row for one meet. Refuses an empty meet."""
    meet = require_text(meet, "meet")
    return conn.execute(
        "DELETE FROM athletes WHERE meet = %s RETURNING 1",
        (meet,),
    ).rowcount


def delete_session_schedule_by_meet(conn, meet: Any) -> int:
    """Delete every schedule row for one meet. Refuses an empty meet."""
    meet = require_text(meet, "meet")
    return conn.execute(
        "DELETE FROM session_schedule WHERE meet = %s RETURNING 1",
        (meet,),
    ).rowcount


def _plan_exact_set_sync(
    existing_by_id: dict[Any, dict[str, Any]],
    existing_by_key: dict[Any, dict[str, Any]],
    prepared: Iterable[tuple[dict[str, Any], Any, Any, dict[str, Any]]],
    duplicate_message: str,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], dict[str, int]]:
    """The one copy of the exact-set replace policy.

    ``prepared`` yields ``(row, key, convex_id, values)`` per incoming row. Rows
    that disappeared from the payload are deleted, the rest are upserted, and
    rows whose values are unchanged are skipped. Planning is complete before the
    caller writes anything, so a duplicate key in the payload aborts the whole
    sync without having deleted a single row.
    """
    inserted = 0
    updated = 0
    unchanged = 0
    incoming_keys: set[Any] = set()
    rows_to_write: list[dict[str, Any]] = []

    for row, key, convex_id, values in prepared:
        if key in incoming_keys:
            raise ValueError(f"{duplicate_message}: {key}")
        incoming_keys.add(key)
        existing = existing_by_id.get(convex_id) or existing_by_key.get(key)
        if existing is None:
            inserted += 1
            rows_to_write.append(row)
        elif row_changed(existing, values):
            updated += 1
            rows_to_write.append(row)
        else:
            unchanged += 1

    rows_to_delete = [row for key, row in existing_by_key.items() if key not in incoming_keys]
    return (
        rows_to_write,
        rows_to_delete,
        {
            "inserted": inserted,
            "updated": updated,
            "unchanged": unchanged,
            "deleted": len(rows_to_delete),
        },
    )


def upsert_lifting_result(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    legacy_id = first(row, "legacyId", "legacy_id")
    event_id = first(row, "eventId", "event_id", default="")
    meet = first(row, "meet", default="")
    date = first(row, "date", default="")
    name = first(row, "name", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id("lifting_result", event_id, meet, name)
    # Lookup precedence when several rows could match: the explicit identity
    # (convex_id) wins over the Convex-era legacy_id, which wins over the
    # natural key. The ORDER BY below makes that deterministic; without it
    # `LIMIT 1` picks whichever row the planner reaches first.
    values = {
        "legacy_id": legacy_id,
        "event_id": event_id,
        "meet": meet,
        "date": date,
        "name": name,
        "age": first(row, "age"),
        "body_weight": first(row, "bodyWeight", "body_weight"),
        "snatch1": first(row, "snatch1"),
        "snatch2": first(row, "snatch2"),
        "snatch3": first(row, "snatch3"),
        "snatch_best": first(row, "snatchBest", "snatch_best"),
        "cj1": first(row, "cj1"),
        "cj2": first(row, "cj2"),
        "cj3": first(row, "cj3"),
        "cj_best": first(row, "cjBest", "cj_best"),
        "total": first(row, "total"),
        "adaptive": bool(first(row, "adaptive", default=False)),
        "federation": normalize_federation(first(row, "federation")),
    }
    existing = conn.execute(
        """
        SELECT id, convex_id, legacy_id, event_id, meet, date, name, age, body_weight,
            snatch1, snatch2, snatch3, snatch_best, cj1, cj2, cj3, cj_best,
            total, adaptive, federation
        FROM lifting_results
        WHERE convex_id = %s
            OR (legacy_id IS NOT DISTINCT FROM %s AND legacy_id IS NOT NULL)
            OR (event_id = %s AND meet = %s AND name = %s)
        ORDER BY
            CASE
                WHEN convex_id = %s THEN 0
                WHEN legacy_id IS NOT DISTINCT FROM %s AND legacy_id IS NOT NULL THEN 1
                ELSE 2
            END
        LIMIT 1
        """,
        (convex_id, legacy_id, event_id, meet, name, convex_id, legacy_id),
    ).fetchone()
    if existing:
        convex_id = existing["convex_id"]
    was_changed = row_changed(existing, values)

    result = conn.execute(
        """
        INSERT INTO lifting_results (
            convex_id, legacy_id, event_id, meet, date, name, age, body_weight,
            snatch1, snatch2, snatch3, snatch_best, cj1, cj2, cj3, cj_best,
            total, adaptive, federation
        )
        VALUES (
            %(convex_id)s, %(legacy_id)s, %(event_id)s, %(meet)s, %(date)s, %(name)s, %(age)s,
            %(body_weight)s, %(snatch1)s, %(snatch2)s, %(snatch3)s, %(snatch_best)s,
            %(cj1)s, %(cj2)s, %(cj3)s, %(cj_best)s, %(total)s, %(adaptive)s, %(federation)s
        )
        ON CONFLICT (convex_id) DO UPDATE SET
            legacy_id = EXCLUDED.legacy_id,
            event_id = EXCLUDED.event_id,
            meet = EXCLUDED.meet,
            date = EXCLUDED.date,
            name = EXCLUDED.name,
            age = EXCLUDED.age,
            body_weight = EXCLUDED.body_weight,
            snatch1 = EXCLUDED.snatch1,
            snatch2 = EXCLUDED.snatch2,
            snatch3 = EXCLUDED.snatch3,
            snatch_best = EXCLUDED.snatch_best,
            cj1 = EXCLUDED.cj1,
            cj2 = EXCLUDED.cj2,
            cj3 = EXCLUDED.cj3,
            cj_best = EXCLUDED.cj_best,
            total = EXCLUDED.total,
            adaptive = EXCLUDED.adaptive,
            federation = EXCLUDED.federation
        RETURNING id
        """,
        {"convex_id": convex_id, **values},
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": existing is None, "wasChanged": was_changed}


def upsert_record(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    record_type = first(row, "recordType", "record_type", default="")
    age_category = normalize_age_category(first(row, "ageCategory", "age_category", default=""))
    gender = normalize_gender(first(row, "gender", default=""))
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "record", record_type, age_category, gender, weight_class
    )
    values = {
        "record_type": record_type,
        "age_category": age_category,
        "gender": gender,
        "weight_class": weight_class,
        "snatch_record": first(row, "snatchRecord", "snatch_record"),
        "cj_record": first(row, "cjRecord", "cj_record"),
        "total_record": first(row, "totalRecord", "total_record"),
    }
    existing = conn.execute(
        """
        SELECT id, convex_id, record_type, age_category, gender, weight_class,
            snatch_record, cj_record, total_record
        FROM records
        WHERE convex_id = %s
            OR (record_type = %s AND age_category = %s AND gender = %s AND weight_class = %s)
        LIMIT 1
        """,
        (convex_id, record_type, age_category, gender, weight_class),
    ).fetchone()
    if existing:
        convex_id = existing["convex_id"]
    was_changed = row_changed(existing, values)
    result = conn.execute(
        """
        INSERT INTO records (
            convex_id, record_type, age_category, gender, weight_class,
            snatch_record, cj_record, total_record
        )
        VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
        ON CONFLICT (convex_id) DO UPDATE SET
            record_type = EXCLUDED.record_type,
            age_category = EXCLUDED.age_category,
            gender = EXCLUDED.gender,
            weight_class = EXCLUDED.weight_class,
            snatch_record = EXCLUDED.snatch_record,
            cj_record = EXCLUDED.cj_record,
            total_record = EXCLUDED.total_record
        RETURNING id
        """,
        (
            convex_id,
            values["record_type"],
            values["age_category"],
            values["gender"],
            values["weight_class"],
            values["snatch_record"],
            values["cj_record"],
            values["total_record"],
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": existing is None, "wasChanged": was_changed}


def upsert_wso_record(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    wso = first(row, "wso", default="")
    age_category = normalize_age_category(first(row, "ageCategory", "age_category", default=""))
    gender = normalize_gender(first(row, "gender", default=""))
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "wso_record", wso, age_category, gender, weight_class
    )
    values = {
        "wso": wso,
        "age_category": age_category,
        "gender": gender,
        "weight_class": weight_class,
        "snatch_record": first(row, "snatchRecord", "snatch_record"),
        "cj_record": first(row, "cjRecord", "cj_record"),
        "total_record": first(row, "totalRecord", "total_record"),
    }
    existing = conn.execute(
        """
        SELECT id, convex_id, wso, age_category, gender, weight_class,
            snatch_record, cj_record, total_record
        FROM wso_records
        WHERE convex_id = %s
            OR (wso = %s AND age_category = %s AND gender = %s AND weight_class = %s)
        LIMIT 1
        """,
        (convex_id, wso, age_category, gender, weight_class),
    ).fetchone()
    if existing:
        convex_id = existing["convex_id"]
    was_changed = row_changed(existing, values)
    result = conn.execute(
        """
        INSERT INTO wso_records (
            convex_id, wso, age_category, gender, weight_class,
            snatch_record, cj_record, total_record
        )
        VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
        ON CONFLICT (convex_id) DO UPDATE SET
            wso = EXCLUDED.wso,
            age_category = EXCLUDED.age_category,
            gender = EXCLUDED.gender,
            weight_class = EXCLUDED.weight_class,
            snatch_record = EXCLUDED.snatch_record,
            cj_record = EXCLUDED.cj_record,
            total_record = EXCLUDED.total_record
        RETURNING id
        """,
        (
            convex_id,
            values["wso"],
            values["age_category"],
            values["gender"],
            values["weight_class"],
            values["snatch_record"],
            values["cj_record"],
            values["total_record"],
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": existing is None, "wasChanged": was_changed}


def upsert_standard(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    age_category = normalize_age_category(first(row, "ageCategory", "age_category", default=""))
    gender = normalize_gender(first(row, "gender", default=""))
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "standard", age_category, gender, weight_class
    )
    values = {
        "age_category": age_category,
        "gender": gender,
        "weight_class": weight_class,
        "standard_a": first(row, "standardA", "standard_a", default=0),
        "standard_b": first(row, "standardB", "standard_b", default=0),
    }
    existing = conn.execute(
        """
        SELECT id, convex_id, age_category, gender, weight_class, standard_a, standard_b
        FROM standards
        WHERE convex_id = %s
            OR (age_category = %s AND gender = %s AND weight_class = %s)
        LIMIT 1
        """,
        (convex_id, age_category, gender, weight_class),
    ).fetchone()
    if existing:
        convex_id = existing["convex_id"]
    was_changed = row_changed(existing, values)
    result = conn.execute(
        """
        INSERT INTO standards (
            convex_id, age_category, gender, weight_class, standard_a, standard_b
        )
        VALUES (%s, %s, %s, %s, %s, %s)
        ON CONFLICT (convex_id) DO UPDATE SET
            age_category = EXCLUDED.age_category,
            gender = EXCLUDED.gender,
            weight_class = EXCLUDED.weight_class,
            standard_a = EXCLUDED.standard_a,
            standard_b = EXCLUDED.standard_b
        RETURNING id
        """,
        (
            convex_id,
            values["age_category"],
            values["gender"],
            values["weight_class"],
            values["standard_a"],
            values["standard_b"],
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": existing is None, "wasChanged": was_changed}


def upsert_qualifying_total(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    event_name = first(row, "eventName", "event_name", default="")
    age_category = normalize_age_category(first(row, "ageCategory", "age_category", default=""))
    gender = normalize_gender(first(row, "gender", default=""))
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "qualifying_total", event_name, age_category, gender, weight_class
    )
    values = {
        "event_name": event_name,
        "gender": gender,
        "age_category": age_category,
        "weight_class": weight_class,
        "qualifying_total": first(row, "qualifyingTotal", "qualifying_total", default=0),
    }
    existing = conn.execute(
        """
        SELECT id, convex_id, event_name, gender, age_category, weight_class, qualifying_total
        FROM qualifying_totals
        WHERE convex_id = %s
            OR (event_name = %s AND gender = %s AND age_category = %s AND weight_class = %s)
        LIMIT 1
        """,
        (convex_id, event_name, gender, age_category, weight_class),
    ).fetchone()
    if existing:
        convex_id = existing["convex_id"]
    was_changed = row_changed(existing, values)
    result = conn.execute(
        """
        INSERT INTO qualifying_totals (
            convex_id, event_name, gender, age_category, weight_class, qualifying_total
        )
        VALUES (%s, %s, %s, %s, %s, %s)
        ON CONFLICT (convex_id) DO UPDATE SET
            event_name = EXCLUDED.event_name,
            gender = EXCLUDED.gender,
            age_category = EXCLUDED.age_category,
            weight_class = EXCLUDED.weight_class,
            qualifying_total = EXCLUDED.qualifying_total
        RETURNING id
        """,
        (
            convex_id,
            values["event_name"],
            values["gender"],
            values["age_category"],
            values["weight_class"],
            values["qualifying_total"],
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": existing is None, "wasChanged": was_changed}


def upsert_meet(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    name = first(row, "name", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id("meet", name)
    values = {
        "name": name,
        "federation": normalize_federation(first(row, "federation", default="USAW")),
        "start_date": first(row, "startDate", "start_date"),
        "end_date": first(row, "endDate", "end_date"),
        "status": first(row, "status", default="upcoming"),
        "time_zone": first(row, "timeZone", "time_zone", default="America/New_York"),
        "updated_at": first(row, "updatedAt", "updated_at", default=millis()),
        "venue_name": first(row, "venueName", "venue_name", default=""),
        "venue_street": first(row, "venueStreet", "venue_street", default=""),
        "venue_city": first(row, "venueCity", "venue_city", default=""),
        "venue_state": first(row, "venueState", "venue_state", default=""),
        "venue_zip": first(row, "venueZip", "venue_zip", default=""),
    }
    existing = conn.execute(
        """
        SELECT id, convex_id, name, federation, start_date, end_date, status, time_zone,
            updated_at, venue_name, venue_street, venue_city, venue_state, venue_zip
        FROM meets
        WHERE convex_id = %s OR name = %s
        LIMIT 1
        """,
        (convex_id, name),
    ).fetchone()
    if existing:
        convex_id = existing["convex_id"]
        # Preserve manually/ops-marked completed; WSO meet-sync always sends upcoming.
        if existing.get("status") == "completed":
            values["status"] = "completed"
    was_changed = row_changed(existing, values)
    result = conn.execute(
        """
        INSERT INTO meets (
            convex_id, name, federation, start_date, end_date, status, time_zone,
            updated_at, venue_name, venue_street, venue_city, venue_state, venue_zip
        )
        VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
        ON CONFLICT (convex_id) DO UPDATE SET
            name = EXCLUDED.name,
            federation = EXCLUDED.federation,
            start_date = EXCLUDED.start_date,
            end_date = EXCLUDED.end_date,
            status = EXCLUDED.status,
            time_zone = EXCLUDED.time_zone,
            updated_at = EXCLUDED.updated_at,
            venue_name = EXCLUDED.venue_name,
            venue_street = EXCLUDED.venue_street,
            venue_city = EXCLUDED.venue_city,
            venue_state = EXCLUDED.venue_state,
            venue_zip = EXCLUDED.venue_zip
        RETURNING id
        """,
        (
            convex_id,
            values["name"],
            values["federation"],
            values["start_date"],
            values["end_date"],
            values["status"],
            values["time_zone"],
            values["updated_at"],
            values["venue_name"],
            values["venue_street"],
            values["venue_city"],
            values["venue_state"],
            values["venue_zip"],
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": existing is None, "wasChanged": was_changed}


def _conflict_kept_existing_session(values: dict[str, Any], result: dict[str, Any] | None) -> bool:
    if not result:
        return False
    kept = {
        "session_number": result.get("session_number"),
        "session_platform": result.get("session_platform"),
    }
    incoming = {
        "session_number": values.get("session_number"),
        "session_platform": values.get("session_platform"),
    }
    return athlete_has_session_assignment(kept) and not athlete_has_session_assignment(incoming)


def athlete_has_session_assignment(row: dict[str, Any] | None) -> bool:
    if not row:
        return False
    if row.get("session_number") is not None:
        return True
    platform = row.get("session_platform")
    if platform is None:
        return False
    if isinstance(platform, str):
        return bool(platform.strip())
    return True


_ATHLETE_SESSION_ASSIGNED_SQL = (
    "athletes.session_number IS NOT NULL"
    " OR (athletes.session_platform IS NOT NULL AND BTRIM(athletes.session_platform) <> '')"
)


def _athlete_upsert_sql(*, preserve_assigned_session: bool) -> str:
    if preserve_assigned_session:
        session_number_set = (
            "session_number = CASE WHEN "
            f"{_ATHLETE_SESSION_ASSIGNED_SQL} "
            "THEN athletes.session_number ELSE EXCLUDED.session_number END"
        )
        session_platform_set = (
            "session_platform = CASE WHEN "
            f"{_ATHLETE_SESSION_ASSIGNED_SQL} "
            "THEN athletes.session_platform ELSE EXCLUDED.session_platform END"
        )
    else:
        session_number_set = "session_number = EXCLUDED.session_number"
        session_platform_set = "session_platform = EXCLUDED.session_platform"
    return f"""
        INSERT INTO athletes (
            convex_id, member_id, name, age, club, wso, gender, weight_class,
            entry_total, session_number, session_platform, meet, adaptive
        )
        VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
        ON CONFLICT (convex_id) DO UPDATE SET
            member_id = EXCLUDED.member_id,
            name = EXCLUDED.name,
            age = EXCLUDED.age,
            club = EXCLUDED.club,
            wso = EXCLUDED.wso,
            gender = EXCLUDED.gender,
            weight_class = EXCLUDED.weight_class,
            entry_total = EXCLUDED.entry_total,
            {session_number_set},
            {session_platform_set},
            meet = EXCLUDED.meet,
            adaptive = EXCLUDED.adaptive
        RETURNING id, session_number, session_platform
        """


def upsert_athlete(
    conn, row: dict[str, Any], *, preserve_assigned_session: bool = False
) -> dict[str, Any]:
    row = clean(row)
    member_id = first(row, "memberId", "member_id", default="")
    name = first(row, "name", default="")
    meet = first(row, "meet", default="")
    # An athlete without a membership number (blank or a `noid:` placeholder
    # from the entry scraper) is identified by (meet, normalized name) so every
    # nightly re-scrape updates the same row instead of minting a new one.
    idless = is_placeholder_member_id(member_id)
    if idless:
        convex_id = first(row, "convexId", "convex_id") or stable_id(
            "athlete", meet, "noid", normalize_name(name)
        )
    else:
        convex_id = first(row, "convexId", "convex_id") or stable_id("athlete", meet, member_id, name)
    values = {
        "member_id": member_id,
        "name": name,
        "age": first(row, "age", default=0),
        "club": first(row, "club", default=""),
        "wso": first(row, "wso"),
        "gender": normalize_gender(first(row, "gender", default="")),
        "weight_class": first(row, "weightClass", "weight_class", default=""),
        "entry_total": first(row, "entryTotal", "entry_total", default=0),
        "session_number": first(row, "sessionNumber", "session_number"),
        "session_platform": normalize_platform(first(row, "sessionPlatform", "session_platform")),
        "meet": meet,
        "adaptive": bool(first(row, "adaptive", default=False)),
    }
    if idless:
        lookup_sql = f"""
            SELECT id, convex_id, member_id, name, age, club, wso, gender, weight_class,
                entry_total, session_number, session_platform, meet, adaptive
            FROM athletes
            WHERE convex_id = %s
                OR (
                    meet = %s
                    AND (member_id = '' OR member_id LIKE 'noid:%%')
                    AND {NORMALIZED_NAME_SQL} = %s
                )
            LIMIT 1
        """
        lookup_params = (convex_id, meet, normalize_name(name))
    else:
        lookup_sql = """
            SELECT id, convex_id, member_id, name, age, club, wso, gender, weight_class,
                entry_total, session_number, session_platform, meet, adaptive
            FROM athletes
            WHERE convex_id = %s
                OR (meet = %s AND member_id = %s AND name = %s)
            LIMIT 1
        """
        lookup_params = (convex_id, meet, member_id, name)
    if preserve_assigned_session:
        lookup_sql += " FOR UPDATE"
    existing = conn.execute(lookup_sql, lookup_params).fetchone()
    if existing:
        convex_id = existing["convex_id"]
        if preserve_assigned_session and athlete_has_session_assignment(existing):
            logger.warning(
                "skipped athlete upsert because session already set meet=%s id=%s convex_id=%s",
                existing.get("meet"),
                existing.get("id"),
                existing.get("convex_id"),
            )
            return {
                "id": str(existing["id"]),
                "wasInsert": False,
                "wasChanged": False,
                "skipped": True,
                "skipReason": "session already set",
            }
    was_changed = row_changed(existing, values)
    result = conn.execute(
        _athlete_upsert_sql(preserve_assigned_session=preserve_assigned_session),
        (
            convex_id,
            values["member_id"],
            values["name"],
            values["age"],
            values["club"],
            values["wso"],
            values["gender"],
            values["weight_class"],
            values["entry_total"],
            values["session_number"],
            values["session_platform"],
            values["meet"],
            values["adaptive"],
        ),
    ).fetchone()
    if preserve_assigned_session and _conflict_kept_existing_session(values, result):
        logger.warning(
            "entry upsert kept existing session meet=%s id=%s convex_id=%s",
            values["meet"],
            result.get("id"),
            convex_id,
        )
    return {"id": str(result["id"]), "wasInsert": existing is None, "wasChanged": was_changed}


def upsert_session_schedule(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    meet = first(row, "meet", default="")
    session_id = first(row, "sessionId", "session_id", default=0)
    # Platform casing and time format are canonicalised here, at the ingest
    # boundary, so the app never sees "red" or "09:00:00" next to "Red" and
    # "9:00 AM". Unparseable times are kept verbatim (the validator warns).
    platform = normalize_platform(first(row, "platform", default=""))
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "session_schedule", meet, session_id, platform, weight_class
    )
    values = {
        "date": first(row, "date", default=""),
        "session_id": session_id,
        "start_time": normalize_time(first(row, "startTime", "start_time", default="")),
        "weigh_in_time": normalize_time(first(row, "weighInTime", "weigh_in_time", default="")),
        "platform": platform,
        "weight_class": weight_class,
        "meet": meet,
    }
    existing = conn.execute(
        """
        SELECT id, convex_id, date, session_id, start_time, weigh_in_time, platform, weight_class, meet
        FROM session_schedule
        WHERE convex_id = %s
            OR (meet = %s AND session_id = %s AND lower(platform) = lower(%s) AND weight_class = %s)
        LIMIT 1
        """,
        (convex_id, meet, session_id, platform, weight_class),
    ).fetchone()
    if existing:
        convex_id = existing["convex_id"]
    was_changed = row_changed(existing, values)
    result = conn.execute(
        """
        INSERT INTO session_schedule (
            convex_id, date, session_id, start_time, weigh_in_time, platform, weight_class, meet
        )
        VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
        ON CONFLICT (convex_id) DO UPDATE SET
            date = EXCLUDED.date,
            session_id = EXCLUDED.session_id,
            start_time = EXCLUDED.start_time,
            weigh_in_time = EXCLUDED.weigh_in_time,
            platform = EXCLUDED.platform,
            weight_class = EXCLUDED.weight_class,
            meet = EXCLUDED.meet
        RETURNING id
        """,
        (
            convex_id,
            values["date"],
            values["session_id"],
            values["start_time"],
            values["weigh_in_time"],
            values["platform"],
            values["weight_class"],
            values["meet"],
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": existing is None, "wasChanged": was_changed}


def upsert_intl_ranking(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    meet = first(row, "meet", default="")
    gender = normalize_gender(first(row, "gender", default=""))
    age_category = normalize_age_category(first(row, "ageCategory", "age_category", default=""))
    ranking = first(row, "ranking", default=0)
    name = first(row, "name", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "intl_ranking", meet, gender, age_category, ranking, name
    )
    values = {
        "legacy_id": first(row, "legacyId", "legacy_id"),
        "meet": meet,
        "ranking": ranking,
        "name": name,
        "weight_class": first(row, "weightClass", "weight_class"),
        "total": first(row, "total"),
        "percent_a": first(row, "percentA", "percent_a"),
        "gender": gender,
        "age_category": age_category,
    }
    existing = conn.execute(
        """
        SELECT id, convex_id, legacy_id, meet, ranking, name, weight_class,
            total, percent_a, gender, age_category
        FROM intl_rankings
        WHERE convex_id = %s
            OR (meet = %s AND gender = %s AND age_category = %s AND ranking = %s AND name = %s)
        LIMIT 1
        """,
        (convex_id, meet, gender, age_category, ranking, name),
    ).fetchone()
    if existing:
        convex_id = existing["convex_id"]
    was_changed = row_changed(existing, values)
    result = conn.execute(
        """
        INSERT INTO intl_rankings (
            convex_id, legacy_id, meet, ranking, name, weight_class,
            total, percent_a, gender, age_category
        )
        VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
        ON CONFLICT (convex_id) DO UPDATE SET
            legacy_id = EXCLUDED.legacy_id,
            meet = EXCLUDED.meet,
            ranking = EXCLUDED.ranking,
            name = EXCLUDED.name,
            weight_class = EXCLUDED.weight_class,
            total = EXCLUDED.total,
            percent_a = EXCLUDED.percent_a,
            gender = EXCLUDED.gender,
            age_category = EXCLUDED.age_category
        RETURNING id
        """,
        (
            convex_id,
            values["legacy_id"],
            values["meet"],
            values["ranking"],
            values["name"],
            values["weight_class"],
            values["total"],
            values["percent_a"],
            values["gender"],
            values["age_category"],
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": existing is None, "wasChanged": was_changed}


def replace_records(conn, record_type: str, rows: Iterable[dict[str, Any]]) -> dict[str, Any]:
    """Wholesale replace of one record type (e.g. IWF world records).

    Refuses an empty ``record_type`` so the DELETE always has a key, and an
    empty payload so a failed scrape cannot wipe the set -- the same rule
    ``replace_all_intl_rankings`` enforces.
    """
    record_type = require_text(record_type, "recordType")
    rows = list(rows)
    if not rows:
        raise ValueError(
            f"refusing to replace {record_type} records with an empty payload"
        )
    conn.execute("DELETE FROM records WHERE record_type = %s", (record_type,))
    inserted = 0
    for row in rows:
        upsert_record(conn, {**row, "recordType": record_type})
        inserted += 1
    return {"deleted": True, "inserted": inserted}


def replace_wso_records(conn, wso: str, rows: Iterable[dict[str, Any]]) -> dict[str, Any]:
    """Exact-set sync of one WSO's record set.

    Refuses an empty ``wso`` so the scan always has a key, and an empty payload
    because an exact-set sync treats "no incoming rows" as "every existing row
    disappeared" -- a failed PDF parse would silently delete the WSO's whole
    record set. Same rule ``replace_records`` enforces.
    """
    require_text(wso, "wso")
    prepared_rows = [{**row, "wso": wso} for row in rows]
    if not prepared_rows:
        raise ValueError(f"refusing to replace {wso} WSO records with an empty payload")
    existing_rows = conn.execute(
        """
        SELECT convex_id, wso, age_category, gender, weight_class,
            snatch_record, cj_record, total_record
        FROM wso_records
        WHERE wso = %s
        """,
        (wso,),
    ).fetchall()
    existing_by_id = {row["convex_id"]: row for row in existing_rows}
    existing_by_key = {
        (row["wso"], row["age_category"], row["gender"], row["weight_class"]): row
        for row in existing_rows
    }

    def prepare():
        for row in prepared_rows:
            age_category = normalize_age_category(
                first(row, "ageCategory", "age_category", default="")
            )
            gender = normalize_gender(first(row, "gender", default=""))
            weight_class = first(row, "weightClass", "weight_class", default="")
            key = (wso, age_category, gender, weight_class)
            convex_id = first(row, "convexId", "convex_id") or stable_id(
                "wso_record", wso, age_category, gender, weight_class
            )
            values = {
                "wso": wso,
                "age_category": age_category,
                "gender": gender,
                "weight_class": weight_class,
                "snatch_record": first(row, "snatchRecord", "snatch_record"),
                "cj_record": first(row, "cjRecord", "cj_record"),
                "total_record": first(row, "totalRecord", "total_record"),
            }
            yield row, key, convex_id, values

    rows_to_write, rows_to_delete, counts = _plan_exact_set_sync(
        existing_by_id, existing_by_key, prepare(), "Duplicate WSO record in payload"
    )
    for row in rows_to_delete:
        conn.execute(
            "DELETE FROM wso_records WHERE convex_id = %s",
            (row["convex_id"],),
        )
    for row in rows_to_write:
        upsert_wso_record(conn, row)
    return counts


def replace_intl_rankings_group(conn, args: dict[str, Any]) -> dict[str, Any]:
    meet = first(args, "meet", default="")
    gender = normalize_gender(first(args, "gender", default=""))
    age_category = normalize_age_category(first(args, "ageCategory", "age_category", default=""))
    require_text(meet, "meet")
    require_text(gender, "gender")
    require_text(age_category, "ageCategory")
    rankings = args.get("rankings", [])
    if not isinstance(rankings, list):
        raise ValueError("rankings must be a list")
    # An exact-set sync with no incoming rows deletes the whole group. Removing
    # a group that genuinely disappeared is `deleteMissingIntlRankingGroups`'
    # job, so an empty payload here is a failed scrape, not an empty group.
    if not rankings:
        raise ValueError(
            f"refusing to replace intl rankings for {meet}/{gender}/{age_category} "
            "with an empty payload"
        )
    existing_rows = conn.execute(
        """
        SELECT convex_id, legacy_id, meet, ranking, name, weight_class,
            total, percent_a, gender, age_category
        FROM intl_rankings
        WHERE meet = %s AND gender = %s AND age_category = %s
        """,
        (meet, gender, age_category),
    ).fetchall()
    existing_by_id = {row["convex_id"]: row for row in existing_rows}
    existing_by_key = {
        (row["meet"], row["gender"], row["age_category"], row["ranking"], row["name"]): row
        for row in existing_rows
    }
    def prepare():
        for row in rankings:
            ranking = first(row, "ranking", default=0)
            name = first(row, "name", default="")
            key = (meet, gender, age_category, ranking, name)
            convex_id = first(row, "convexId", "convex_id") or stable_id(
                "intl_ranking", meet, gender, age_category, ranking, name
            )
            values = {
                "legacy_id": first(row, "legacyId", "legacy_id"),
                "meet": meet,
                "ranking": ranking,
                "name": name,
                "weight_class": first(row, "weightClass", "weight_class"),
                "total": first(row, "total"),
                "percent_a": first(row, "percentA", "percent_a"),
                "gender": gender,
                "age_category": age_category,
            }
            yield row, key, convex_id, values

    rows_to_write, rows_to_delete, counts = _plan_exact_set_sync(
        existing_by_id, existing_by_key, prepare(), "Duplicate intl ranking in payload"
    )
    for row in rows_to_delete:
        conn.execute(
            "DELETE FROM intl_rankings WHERE convex_id = %s",
            (row["convex_id"],),
        )
    for row in rows_to_write:
        upsert_intl_ranking(
            conn, {**row, "meet": meet, "gender": gender, "ageCategory": age_category}
        )
    return counts


def replace_all_intl_rankings(conn, rankings: Iterable[dict[str, Any]]) -> dict[str, Any]:
    """Wholesale replace of every intl ranking. Refuses an empty payload so a
    failed scrape cannot wipe the table."""
    rankings = list(rankings)
    if not rankings:
        raise ValueError("refusing to replace all intl rankings with an empty payload")
    conn.execute("DELETE FROM intl_rankings")
    for row in rankings:
        upsert_intl_ranking(conn, row)
    return {"inserted": len(rankings)}


def delete_missing_intl_ranking_groups(conn, groups: Iterable[dict[str, Any]]) -> dict[str, Any]:
    active: set[tuple[str, str, str]] = set()
    for group in groups:
        meet = first(group, "meet", default="")
        gender = normalize_gender(first(group, "gender", default=""))
        age_category = normalize_age_category(
            first(group, "ageCategory", "age_category", default="")
        )
        if meet and gender and age_category:
            active.add((meet, gender, age_category))

    if not active:
        return {"deletedGroups": [], "deleted": 0}

    existing_groups = conn.execute(
        "SELECT DISTINCT meet, gender, age_category FROM intl_rankings"
    ).fetchall()
    deleted_groups: list[dict[str, Any]] = []
    for row in existing_groups:
        key = (row["meet"], row["gender"], row["age_category"])
        if key in active:
            continue
        deleted = conn.execute(
            "DELETE FROM intl_rankings WHERE meet = %s AND gender = %s AND age_category = %s",
            key,
        ).rowcount
        deleted_groups.append(
            {
                "meet": key[0],
                "gender": key[1],
                "ageCategory": key[2],
                "deleted": deleted,
            }
        )
    return {
        "deletedGroups": deleted_groups,
        "deleted": sum(int(group["deleted"]) for group in deleted_groups),
    }
