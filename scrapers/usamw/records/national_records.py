#!/usr/bin/env python3
from __future__ import annotations

import argparse
import html
import io
import os
import re
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

import pdfplumber
import requests

SCRAPERS_DIR = Path(__file__).resolve().parents[2]
if str(SCRAPERS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRAPERS_DIR))

from common import postgres_writer as pg  # noqa: E402

PAGE_URL = "https://usamasters.net/masters-records-grand-slam"
RECORD_TYPE = "USAMW"
INGEST_PATH = "scraperIngestion:ingestRecord"
REQUEST_TIMEOUT_SECONDS = 45
REQUEST_HEADERS = {
    "User-Agent": (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
        "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36"
    )
}


def fetch_text(url: str) -> str:
    response = requests.get(url, headers=REQUEST_HEADERS, timeout=REQUEST_TIMEOUT_SECONDS)
    response.raise_for_status()
    return response.text


def fetch_bytes(url: str) -> bytes:
    response = requests.get(url, headers=REQUEST_HEADERS, timeout=REQUEST_TIMEOUT_SECONDS)
    response.raise_for_status()
    return response.content


def discover_national_record_pdfs(page_url: str = PAGE_URL) -> dict[str, str]:
    """Return current national records PDF URLs keyed by Men/Women."""
    page = html.unescape(fetch_text(page_url))
    urls: dict[str, str] = {}

    for href in re.findall(r'href="([^"]+)"', page, flags=re.I):
        decoded = html.unescape(href)
        lower = decoded.lower()
        if "storage.googleapis.com" not in lower or "filename=nm" not in lower or ".pdf" not in lower:
            continue
        if re.search(r"[-_ ]men\.pdf\b", lower):
            urls["Men"] = decoded
        elif re.search(r"[-_ ]women\.pdf\b", lower):
            urls["Women"] = decoded

    missing = {"Men", "Women"} - set(urls)
    if missing:
        raise RuntimeError(f"Could not find national records PDFs for: {', '.join(sorted(missing))}")
    return urls


def normalize_age_category(text: str) -> str | None:
    match = re.search(r"\b[MW]\s*(\d+)\s*-\s*\d+\b", text)
    if match:
        return f"Masters {match.group(1)}"
    match = re.search(r"\b[MW]\s*(\d+)\+", text)
    if match:
        return f"Masters {match.group(1)}"
    return None


def format_weight_class(value: str) -> str | None:
    match = re.search(r"(\d+\+?)", value)
    if not match:
        return None
    return f"{match.group(1)}kg"


def blank_record(age_category: str, gender: str, weight_class: str) -> dict[str, Any]:
    return {
        "recordType": RECORD_TYPE,
        "ageCategory": age_category,
        "gender": gender,
        "weightClass": weight_class,
        "snatchRecord": None,
        "cjRecord": None,
        "totalRecord": None,
    }


def parse_records_pdf(pdf_bytes: bytes, gender: str) -> list[dict[str, Any]]:
    by_key: dict[tuple[str, str, str], dict[str, Any]] = {}

    with pdfplumber.open(io.BytesIO(pdf_bytes)) as pdf:
        for page in pdf.pages:
            text = page.extract_text(x_tolerance=2, y_tolerance=2) or ""
            lines = [line.strip() for line in text.splitlines() if line.strip()]
            age_category = None
            for line in lines[:8]:
                maybe_age = normalize_age_category(line)
                if maybe_age:
                    age_category = maybe_age
                    break
            if not age_category:
                continue

            for line in lines:
                match = re.match(
                    r"^(?P<cat>\d+\+?)\s+(?P<lift>SNA|SNATCH|CNJ|C&J|CLEAN|TOT|TOTAL)\s+(?P<record>\d+(?:\.\d+)?)\b",
                    line,
                    flags=re.I,
                )
                if not match:
                    continue

                weight_class = format_weight_class(match.group("cat"))
                if not weight_class:
                    continue

                key = (age_category, gender, weight_class)
                record = by_key.setdefault(key, blank_record(age_category, gender, weight_class))
                value = int(float(match.group("record")))
                lift = match.group("lift").upper()
                if lift.startswith("SNA") or lift.startswith("SNATCH"):
                    record["snatchRecord"] = value
                elif lift in {"CNJ", "C&J"} or lift.startswith("CLEAN"):
                    record["cjRecord"] = value
                elif lift.startswith("TOT") or lift.startswith("TOTAL"):
                    record["totalRecord"] = value

    return sorted(
        by_key.values(),
        key=lambda row: (
            row["gender"],
            int(re.search(r"\d+", row["ageCategory"]).group(0)),
            float(row["weightClass"].replace("+kg", ".5").replace("kg", "")),
        ),
    )


def scrape_records(page_url: str = PAGE_URL) -> tuple[dict[str, str], list[dict[str, Any]]]:
    urls = discover_national_record_pdfs(page_url)
    rows: list[dict[str, Any]] = []
    for gender, url in urls.items():
        print(f"Downloading {gender} national records PDF: {url}")
        parsed = parse_records_pdf(fetch_bytes(url), gender)
        print(f"Parsed {len(parsed)} {gender} records")
        rows.extend(parsed)
    return urls, rows


def summarize(results: list[dict[str, Any]]) -> dict[str, int]:
    stats = {"inserted": 0, "updated": 0, "unchanged": 0}
    for result in results:
        if result.get("wasInsert"):
            stats["inserted"] += 1
        elif result.get("wasChanged"):
            stats["updated"] += 1
        else:
            stats["unchanged"] += 1
    return stats


def ingest_postgres(rows: list[dict[str, Any]]) -> dict[str, int] | None:
    if not os.getenv("DATABASE_URL"):
        return None
    with pg.connect() as conn:
        results = [pg.upsert_record(conn, row) for row in rows]
        conn.commit()
    return summarize(results)


def convex_action(endpoint: str, args: dict[str, Any]) -> dict[str, Any]:
    response = requests.post(
        endpoint,
        json={"path": INGEST_PATH, "args": args},
        timeout=REQUEST_TIMEOUT_SECONDS,
    )
    response.raise_for_status()
    data = response.json()
    return data.get("value", data) if isinstance(data, dict) else {}


def ingest_convex(rows: list[dict[str, Any]]) -> dict[str, int] | None:
    convex_url = os.getenv("CONVEX_URL") or os.getenv("EXPO_PUBLIC_CONVEX_URL")
    scraper_secret = os.getenv("SCRAPER_SECRET")
    if not convex_url or not convex_url.startswith(("http://", "https://")) or not scraper_secret:
        return None

    endpoint = f"{convex_url.rstrip('/')}/api/action"
    results = [
        convex_action(endpoint, {"scraperSecret": scraper_secret, **row})
        for row in rows
    ]
    return summarize(results)


def ingest(rows: list[dict[str, Any]]) -> dict[str, dict[str, int]]:
    stats: dict[str, dict[str, int]] = {}
    postgres = ingest_postgres(rows)
    if postgres:
        stats["postgres"] = postgres
    convex = ingest_convex(rows)
    if convex:
        stats["convex"] = convex
    if not stats:
        raise RuntimeError("No ingest targets configured. Set DATABASE_URL and/or CONVEX_URL + SCRAPER_SECRET.")
    return stats


def post_slack(text: str) -> None:
    webhook_url = os.getenv("SLACK_USAMW_RECORDS_WEBHOOK_URL") or os.getenv("SLACK_RECORDS_WEBHOOK_URL") or os.getenv("SLACK_WEBHOOK_URL")
    if not webhook_url:
        return
    try:
        requests.post(webhook_url, json={"text": text}, timeout=REQUEST_TIMEOUT_SECONDS)
    except Exception as exc:  # noqa: BLE001
        print(f"Slack notification failed: {exc}", file=sys.stderr)


def format_stats(stats: dict[str, dict[str, int]]) -> str:
    return "\n".join(
        f"{target}: {value['inserted']} inserted, {value['updated']} updated, {value['unchanged']} unchanged"
        for target, value in stats.items()
    )


def has_database_changes(stats: dict[str, dict[str, int]]) -> bool:
    return any(
        value["inserted"] + value["updated"] > 0
        for value in stats.values()
    )


def counts_by_gender(rows: list[dict[str, Any]]) -> dict[str, int]:
    counts: defaultdict[str, int] = defaultdict(int)
    for row in rows:
        counts[row["gender"]] += 1
    return dict(counts)


def main() -> int:
    parser = argparse.ArgumentParser(description="Scrape USAMW national masters records")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--page-url", default=PAGE_URL)
    args = parser.parse_args()

    urls, rows = scrape_records(args.page_url)
    if not rows:
        raise RuntimeError("Parsed 0 USAMW national records")

    counts = counts_by_gender(rows)
    print(f"Parsed {len(rows)} total USAMW records: {counts}")
    print(f"Source PDFs: {urls}")

    if args.dry_run:
        print("DRY RUN: no database changes made")
        for row in rows[:12]:
            print(
                f"  {row['gender']} {row['ageCategory']} {row['weightClass']}: "
                f"S={row['snatchRecord']} CJ={row['cjRecord']} T={row['totalRecord']}"
            )
        return 0

    stats = ingest(rows)
    message = f"USAMW national records update complete\n{len(rows)} parsed records\n{format_stats(stats)}"
    print(message)
    if has_database_changes(stats):
        post_slack(message)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
