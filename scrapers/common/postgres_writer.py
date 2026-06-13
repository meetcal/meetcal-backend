from __future__ import annotations

import hashlib
import os
import time
from contextlib import contextmanager
from typing import Any, Iterable

import psycopg
from psycopg.rows import dict_row


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


def upsert_lifting_result(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    legacy_id = first(row, "legacyId", "legacy_id")
    event_id = first(row, "eventId", "event_id", default="")
    meet = first(row, "meet", default="")
    date = first(row, "date", default="")
    name = first(row, "name", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id("lifting_result", event_id, meet, name)

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
        {
            "convex_id": convex_id,
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
            "federation": first(row, "federation"),
        },
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": True, "wasChanged": True}


def upsert_record(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    record_type = first(row, "recordType", "record_type", default="")
    age_category = first(row, "ageCategory", "age_category", default="")
    gender = first(row, "gender", default="")
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "record", record_type, age_category, gender, weight_class
    )
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
            record_type,
            age_category,
            gender,
            weight_class,
            first(row, "snatchRecord", "snatch_record"),
            first(row, "cjRecord", "cj_record"),
            first(row, "totalRecord", "total_record"),
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": True, "wasChanged": True}


def upsert_wso_record(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    wso = first(row, "wso", default="")
    age_category = first(row, "ageCategory", "age_category", default="")
    gender = first(row, "gender", default="")
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "wso_record", wso, age_category, gender, weight_class
    )
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
            wso,
            age_category,
            gender,
            weight_class,
            first(row, "snatchRecord", "snatch_record"),
            first(row, "cjRecord", "cj_record"),
            first(row, "totalRecord", "total_record"),
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": True, "wasChanged": True}


def upsert_standard(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    age_category = first(row, "ageCategory", "age_category", default="")
    gender = first(row, "gender", default="")
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "standard", age_category, gender, weight_class
    )
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
            age_category,
            gender,
            weight_class,
            first(row, "standardA", "standard_a", default=0),
            first(row, "standardB", "standard_b", default=0),
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": True, "wasChanged": True}


def upsert_qualifying_total(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    event_name = first(row, "eventName", "event_name", default="")
    age_category = first(row, "ageCategory", "age_category", default="")
    gender = first(row, "gender", default="")
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "qualifying_total", event_name, age_category, gender, weight_class
    )
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
            event_name,
            gender,
            age_category,
            weight_class,
            first(row, "qualifyingTotal", "qualifying_total", default=0),
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": True}


def upsert_meet(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    name = first(row, "name", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id("meet", name)
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
            name,
            first(row, "federation", default="USAW"),
            first(row, "startDate", "start_date"),
            first(row, "endDate", "end_date"),
            first(row, "status", default="upcoming"),
            first(row, "timeZone", "time_zone", default="America/New_York"),
            first(row, "updatedAt", "updated_at", default=millis()),
            first(row, "venueName", "venue_name", default=""),
            first(row, "venueStreet", "venue_street", default=""),
            first(row, "venueCity", "venue_city", default=""),
            first(row, "venueState", "venue_state", default=""),
            first(row, "venueZip", "venue_zip", default=""),
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": True}


def upsert_athlete(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    member_id = first(row, "memberId", "member_id", default="")
    name = first(row, "name", default="")
    meet = first(row, "meet", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id("athlete", meet, member_id, name)
    result = conn.execute(
        """
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
            session_number = EXCLUDED.session_number,
            session_platform = EXCLUDED.session_platform,
            meet = EXCLUDED.meet,
            adaptive = EXCLUDED.adaptive
        RETURNING id
        """,
        (
            convex_id,
            member_id,
            name,
            first(row, "age", default=0),
            first(row, "club", default=""),
            first(row, "wso"),
            first(row, "gender", default=""),
            first(row, "weightClass", "weight_class", default=""),
            first(row, "entryTotal", "entry_total", default=0),
            first(row, "sessionNumber", "session_number"),
            first(row, "sessionPlatform", "session_platform"),
            meet,
            bool(first(row, "adaptive", default=False)),
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": True}


def upsert_session_schedule(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    meet = first(row, "meet", default="")
    session_id = first(row, "sessionId", "session_id", default=0)
    platform = first(row, "platform", default="")
    weight_class = first(row, "weightClass", "weight_class", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "session_schedule", meet, session_id, platform, weight_class
    )
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
            first(row, "date", default=""),
            session_id,
            first(row, "startTime", "start_time", default=""),
            first(row, "weighInTime", "weigh_in_time", default=""),
            platform,
            weight_class,
            meet,
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": True}


def upsert_intl_ranking(conn, row: dict[str, Any]) -> dict[str, Any]:
    row = clean(row)
    meet = first(row, "meet", default="")
    gender = first(row, "gender", default="")
    age_category = first(row, "ageCategory", "age_category", default="")
    ranking = first(row, "ranking", default=0)
    name = first(row, "name", default="")
    convex_id = first(row, "convexId", "convex_id") or stable_id(
        "intl_ranking", meet, gender, age_category, ranking, name
    )
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
            first(row, "legacyId", "legacy_id"),
            meet,
            ranking,
            name,
            first(row, "weightClass", "weight_class"),
            first(row, "total"),
            first(row, "percentA", "percent_a"),
            gender,
            age_category,
        ),
    ).fetchone()
    return {"id": str(result["id"]), "wasInsert": True}


def replace_records(conn, record_type: str, rows: Iterable[dict[str, Any]]) -> dict[str, Any]:
    conn.execute("DELETE FROM records WHERE record_type = %s", (record_type,))
    inserted = 0
    for row in rows:
        upsert_record(conn, {**row, "recordType": record_type})
        inserted += 1
    return {"deleted": True, "inserted": inserted}


def replace_wso_records(conn, wso: str, rows: Iterable[dict[str, Any]]) -> dict[str, Any]:
    conn.execute("DELETE FROM wso_records WHERE wso = %s", (wso,))
    inserted = 0
    for row in rows:
        upsert_wso_record(conn, {**row, "wso": wso})
        inserted += 1
    return {"inserted": inserted, "updated": 0, "unchanged": 0, "deleted": True}


def replace_intl_rankings_group(conn, args: dict[str, Any]) -> dict[str, Any]:
    meet = first(args, "meet", default="")
    gender = first(args, "gender", default="")
    age_category = first(args, "ageCategory", "age_category", default="")
    conn.execute(
        "DELETE FROM intl_rankings WHERE meet = %s AND gender = %s AND age_category = %s",
        (meet, gender, age_category),
    )
    inserted = 0
    for row in args.get("rankings", []):
        upsert_intl_ranking(conn, {**row, "meet": meet, "gender": gender, "ageCategory": age_category})
        inserted += 1
    return {"inserted": inserted}
