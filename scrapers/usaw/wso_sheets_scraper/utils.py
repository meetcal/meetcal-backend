"""Shared by the WSO records scrapers: one exact-set write of a WSO's records.

Every scraper parses its source into records ({"wso", "age_category",
"gender", "weight_class", "snatch_record", "cj_record", "total_record"}) and
hands the whole set to ``sync_wso_records``, which replaces the WSO's rows in
Postgres with exactly that set (``scraperIngestion:replaceWSORecordSet``): a
class the source no longer lists is deleted, not left beside the new ones.
So a parse that comes back empty, or with a part (tab, PDF) empty, fails
instead of writing. Same policy as meetcal-app's convex/scrapers/wsoRecords.ts.
"""

from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

from common.postgres_writer import normalize_age_category, normalize_gender

LIFT_FIELDS = (
    ("snatch_record", "snatchRecord"),
    ("cj_record", "cjRecord"),
    ("total_record", "totalRecord"),
)


def every_part(wso: str, parts: Sequence[Tuple[str, List[Dict[str, Any]]]]) -> List[Dict[str, Any]]:
    """The records of a source read in parts, refusing a part that parsed to
    nothing: the sync is an exact set, so one broken tab or PDF would
    otherwise delete its classes while the rest looked fine."""
    empty = [label for label, records in parts if not records]
    if empty:
        raise ValueError(f"{wso}: parsed 0 records from {', '.join(empty)} (layout changed?)")
    return [record for _, records in parts for record in records]


def wso_payload(records: Iterable[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """One ``replaceWSORecordSet`` row per class, the last listed winning, as
    it did when the scrapers upserted row by row (Tennessee-Kentucky's sheet
    lists Senior Women 77 twice). Classes are compared as stored, after age
    and gender are normalized."""
    by_class: Dict[Tuple[str, str, str], Dict[str, Any]] = {}
    for record in records:
        key = (
            normalize_age_category(record["age_category"]),
            normalize_gender(record["gender"]),
            str(record["weight_class"]),
        )
        row: Dict[str, Any] = {
            "ageCategory": record["age_category"],
            "gender": record["gender"],
            "weightClass": str(record["weight_class"]),
        }
        for field, column in LIFT_FIELDS:
            if record.get(field) is not None:
                row[column] = record[field]
        earlier = by_class.pop(key, None)
        if earlier is not None and earlier != row:
            print(f"Warning: {' '.join(key)} listed twice with different values; keeping the last")
        by_class[key] = row
    return list(by_class.values())


def sync_wso_records(
    wso: str,
    records: List[Dict[str, Any]],
    *,
    dry_run: bool = False,
    allow_shrink: bool = False,
) -> Optional[Dict[str, int]]:
    """Replace ``wso``'s Postgres records with exactly ``records``.

    ``allow_shrink`` lets the sync delete more than a quarter of the stored
    classes (``postgres_writer.MAX_WSO_SHRINK_SHARE``), for a source checked
    by hand to have really dropped them.
    """
    if not records:
        raise ValueError(f"{wso}: parsed 0 records (source layout changed?)")
    rows = wso_payload(records)
    if dry_run:
        print(f"Dry run: would sync {len(rows)} {wso} classes")
        return None
    from common.postgres_ingest import IngestClient

    result = IngestClient().action(
        "scraperIngestion:replaceWSORecordSet",
        {"wso": wso, "records": rows, "allowShrink": allow_shrink},
    )
    print(
        f"{wso}: {len(rows)} classes synced: inserted={result['inserted']}, "
        f"updated={result['updated']}, deleted={result['deleted']}, "
        f"unchanged={result['unchanged']}"
    )
    return result
