#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sys
import time
from dataclasses import dataclass
from io import BytesIO
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

import pdfplumber
import requests

SCRAPERS_DIR = Path(__file__).resolve().parents[2]
if str(SCRAPERS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRAPERS_DIR))

from common import postgres_writer as pg  # noqa: E402

REQUEST_TIMEOUT_SECONDS = 45
INGEST_PATH = "scraperIngestion:ingestLiftingResult"


@dataclass
class PdfTextItem:
    text: str
    x: float
    y: float
    is_red: bool


def extract_google_drive_file_id(url: str) -> str | None:
    path_match = re.search(r"/file/d/([a-zA-Z0-9_-]+)", url)
    if path_match:
        return path_match.group(1)
    query_id = parse_qs(urlparse(url).query).get("id")
    return query_id[0] if query_id else None


def fetch_bytes(url: str) -> bytes:
    file_id = extract_google_drive_file_id(url)
    download_url = f"https://drive.google.com/uc?export=download&id={file_id}" if file_id else url
    response = requests.get(download_url, timeout=REQUEST_TIMEOUT_SECONDS)

    if file_id and "text/html" in response.headers.get("content-type", ""):
        confirm = re.search(r"confirm=([^&\"']+)", response.text)
        if not confirm:
            raise RuntimeError(f"Google Drive did not return a PDF for {url}")
        response = requests.get(
            f"{download_url}&confirm={confirm.group(1)}",
            timeout=REQUEST_TIMEOUT_SECONDS,
        )

    response.raise_for_status()
    return response.content


def is_red_color(value: Any) -> bool:
    if not isinstance(value, (list, tuple)) or len(value) < 3:
        return False
    raw_r, raw_g, raw_b = value[:3]
    r = raw_r * 255 if raw_r <= 1 else raw_r
    g = raw_g * 255 if raw_g <= 1 else raw_g
    b = raw_b * 255 if raw_b <= 1 else raw_b
    return r > 200 and g < 100 and b < 100


def word_is_red(word: dict[str, Any]) -> bool:
    chars = word.get("chars") or []
    return any(
        is_red_color(char.get("non_stroking_color"))
        or is_red_color(char.get("stroking_color"))
        for char in chars
    )


def extract_pdf_text_items(pdf_bytes: bytes) -> dict[int, list[PdfTextItem]]:
    pages: dict[int, list[PdfTextItem]] = {}
    with pdfplumber.open(BytesIO(pdf_bytes)) as pdf:
        for page_number, page in enumerate(pdf.pages, start=1):
            words = page.extract_words(
                x_tolerance=2,
                y_tolerance=2,
                keep_blank_chars=False,
                use_text_flow=False,
                return_chars=True,
                extra_attrs=["non_stroking_color", "stroking_color"],
            )
            items = [
                PdfTextItem(
                    text=str(word.get("text", "")).strip(),
                    x=float(word.get("x0", 0)),
                    y=float(word.get("top", 0)),
                    is_red=word_is_red(word),
                )
                for word in words
                if str(word.get("text", "")).strip()
            ]
            items.sort(key=lambda item: (item.y, item.x))
            pages[page_number] = items
    return pages


def to_age_category(age_code: str, weight_category: str) -> str | None:
    match = re.match(r"^([MW])(\d+)$", age_code)
    if not match:
        return None
    gender = "Women's" if match.group(1) == "W" else "Men's"
    start = (int(match.group(2)) // 5) * 5
    return f"{gender} Masters ({start}-{start + 4}) {weight_category}kg"


def to_age_group(age_code: str) -> str | None:
    match = re.match(r"^([MW])(\d+)$", age_code)
    if not match:
        return None
    gender = "Women's" if match.group(1) == "W" else "Men's"
    start = (int(match.group(2)) // 5) * 5
    return f"{gender} Masters ({start}-{start + 4})"


def infer_weight_category(age_code: str, body_weight: float, date: str) -> str | None:
    match = re.match(r"^([MW])\d+$", age_code)
    if not match:
        return None

    after_class_change = date >= "2026-08-01"
    classes = {
        ("M", False): [60, 65, 71, 79, 88, 94, 110],
        ("W", False): [48, 53, 58, 63, 69, 77, 86],
        ("M", True): [60, 65, 70, 75, 85, 95, 110],
        ("W", True): [49, 53, 57, 61, 69, 77, 86],
    }[(match.group(1), after_class_change)]

    for weight_class in classes:
        if body_weight <= weight_class:
            return str(weight_class)
    return f"{classes[-1]}+"


def normalize_name(raw_name: str) -> str | None:
    clean_name = re.sub(r"\s*\(ADT\d*\)|\[ADT\]\s*|\s*\(adaptive\)", " ", raw_name, flags=re.I).strip()
    parts = [part for part in clean_name.split() if part]
    if len(parts) < 2:
        return None

    first_name_index = len(parts) - 1
    for index, part in enumerate(parts):
        if part != part.upper():
            first_name_index = index
            break

    last_name = " ".join(part[:1].upper() + part[1:].lower() for part in parts[:first_name_index])
    first_name = " ".join(parts[first_name_index:])
    return f"{first_name} {last_name}".strip()


def parse_attempt(value: str, is_red: bool) -> int:
    parsed = int(float(value))
    return -parsed if is_red else parsed


def best_positive(values: list[int]) -> int:
    positive = [value for value in values if value > 0]
    return max(positive) if positive else 0


def slug_event_id(meet: str, date: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", meet.strip().lower()).strip("-")
    return f"{slug}-{date}"


def items_on_line(items: list[PdfTextItem], y: float) -> list[PdfTextItem]:
    return [item for item in items if abs(item.y - y) < 2]


def group_lines(items: list[PdfTextItem]) -> list[list[PdfTextItem]]:
    lines: list[list[PdfTextItem]] = []
    for item in sorted(items, key=lambda word: (word.y, word.x)):
        if not lines or abs(lines[-1][0].y - item.y) >= 2:
            lines.append([item])
        else:
            lines[-1].append(item)
    for line in lines:
        line.sort(key=lambda word: word.x)
    return lines


def parse_attempt_item(item: PdfTextItem) -> int:
    if item.text == "-":
        return 0
    return parse_attempt(item.text, item.is_red)


def parse_qmasters_attempt_results_from_lines(
    pages: dict[int, list[PdfTextItem]],
    meet: str,
    date: str,
    adaptive: bool,
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    event_id = slug_event_id(meet, date)

    for items in pages.values():
        current_age_category: str | None = None
        current_age_code: str | None = None

        for line in group_lines(items):
            texts = [item.text for item in line]
            line_text = " ".join(texts)

            age_match = re.search(r"\bAge Group ([MW]\d+)\b", line_text)
            if age_match:
                current_age_code = age_match.group(1)
                current_age_category = to_age_group(current_age_code)
                continue

            if not current_age_category or not current_age_code or len(line) < 13:
                continue
            if not re.match(r"^\d{1,3}$", texts[0]) or not re.match(r"^\d{1,4}$", texts[1]):
                continue

            body_weight_index = next(
                (
                    index
                    for index, item in enumerate(line[2:], start=2)
                    if item.x >= 275 and re.match(r"^\d+\.\d+$", item.text)
                ),
                None,
            )
            if body_weight_index is None or body_weight_index < 4:
                continue

            name_items = [item for item in line[2:body_weight_index] if item.x < 185]
            name = normalize_name(" ".join(item.text for item in name_items))
            if not name:
                continue

            data = line[body_weight_index:]
            if len(data) < 10:
                continue

            try:
                body_weight = float(data[0].text)
                int(data[1].text)
                snatch1 = parse_attempt_item(data[2])
                snatch2 = parse_attempt_item(data[3])
                snatch3 = parse_attempt_item(data[4])
                cj1 = parse_attempt_item(data[5])
                cj2 = parse_attempt_item(data[6])
                cj3 = parse_attempt_item(data[7])
                total = int(float(data[8].text))
            except (ValueError, IndexError):
                continue

            inferred_weight_category = infer_weight_category(current_age_code, body_weight, date)
            age_category = (
                to_age_category(current_age_code, inferred_weight_category)
                if inferred_weight_category
                else current_age_category
            )

            results.append(
                {
                    "adaptive": adaptive,
                    "age": age_category,
                    "bodyWeight": body_weight,
                    "cj1": cj1,
                    "cj2": cj2,
                    "cj3": cj3,
                    "cjBest": best_positive([cj1, cj2, cj3]),
                    "date": date,
                    "eventId": event_id,
                    "federation": "USAMW",
                    "meet": meet,
                    "name": name,
                    "snatch1": snatch1,
                    "snatch2": snatch2,
                    "snatch3": snatch3,
                    "snatchBest": best_positive([snatch1, snatch2, snatch3]),
                    "total": total,
                }
            )

    return results


def parse_results_from_lines(
    pages: dict[int, list[PdfTextItem]],
    meet: str,
    date: str,
    adaptive: bool,
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    event_id = slug_event_id(meet, date)

    for items in pages.values():
        current_age_category: str | None = None
        for line in group_lines(items):
            texts = [item.text for item in line]
            line_text = " ".join(texts)

            age_match = re.search(r"Age Group ([MW]\d+)\s+Weight Category ([\d+]+\+?)", line_text)
            if age_match:
                current_age_category = to_age_category(age_match.group(1), age_match.group(2))
                continue

            if not current_age_category or len(line) < 12:
                continue
            if not re.match(r"^\d{1,4}$", texts[0]) or not re.match(r"^\d{1,4}$", texts[1]):
                continue

            nat_index = next(
                (
                    index
                    for index, text in enumerate(texts[2:], start=2)
                    if re.match(r"^[A-Z]{3}$", text)
                ),
                None,
            )
            if nat_index is None or nat_index <= 2:
                continue

            name = normalize_name(" ".join(texts[2:nat_index]))
            if not name:
                continue

            data = line[nat_index + 1 :]
            if len(data) < 9:
                continue
            if not re.match(r"^\d+\.?\d*$", data[0].text):
                continue

            try:
                snatch1 = parse_attempt_item(data[2])
                snatch2 = parse_attempt_item(data[3])
                snatch3 = parse_attempt_item(data[4])
                cj1 = parse_attempt_item(data[5])
                cj2 = parse_attempt_item(data[6])
                cj3 = parse_attempt_item(data[7])
                total = int(float(data[8].text))
            except (ValueError, IndexError):
                continue

            results.append(
                {
                    "adaptive": adaptive,
                    "age": current_age_category,
                    "bodyWeight": float(data[0].text),
                    "cj1": cj1,
                    "cj2": cj2,
                    "cj3": cj3,
                    "cjBest": best_positive([cj1, cj2, cj3]),
                    "date": date,
                    "eventId": event_id,
                    "federation": "USAMW",
                    "meet": meet,
                    "name": name,
                    "snatch1": snatch1,
                    "snatch2": snatch2,
                    "snatch3": snatch3,
                    "snatchBest": best_positive([snatch1, snatch2, snatch3]),
                    "total": total,
                }
            )

    return results


def parse_results_from_pages(
    pages: dict[int, list[PdfTextItem]],
    meet: str,
    date: str,
    adaptive: bool,
) -> list[dict[str, Any]]:
    line_results = parse_results_from_lines(pages, meet, date, adaptive)
    if line_results:
        return line_results

    qmasters_results = parse_qmasters_attempt_results_from_lines(pages, meet, date, adaptive)
    if qmasters_results:
        return qmasters_results

    results: list[dict[str, Any]] = []
    event_id = slug_event_id(meet, date)

    for items in pages.values():
        current_age_category: str | None = None

        i = 0
        while i < len(items):
            text = items[i].text
            remaining_line = " ".join(item.text for item in items_on_line(items, items[i].y))
            age_match = re.search(r"Age Group ([MW]\d+)\s+Weight Category ([\d+]+\+?)", remaining_line)
            if age_match:
                current_age_category = to_age_category(age_match.group(1), age_match.group(2))

            if not current_age_category or not re.match(r"^\d{1,4}$", text) or i + 1 >= len(items):
                i += 1
                continue

            name_text = items[i + 1].text
            if not re.match(r"^[A-Z]+", name_text):
                i += 1
                continue

            name = normalize_name(name_text)
            if not name:
                i += 1
                continue

            values: list[dict[str, Any]] = []
            j = i + 2
            while j < len(items) and len(values) < 12:
                value_text = items[j].text

                if value_text == "-":
                    values.append({"value": "0", "is_red": False})
                elif re.match(r"^\d+\.?\d*$", value_text):
                    values.append({"value": value_text, "is_red": items[j].is_red})
                elif re.search(r"\d+\.?\d*", value_text):
                    for number in re.findall(r"\d+\.?\d*", value_text):
                        if "." in number or len(number) >= 2:
                            values.append({"value": number, "is_red": items[j].is_red})

                j += 1

                if j < len(items):
                    next_text = items[j].text
                    next_name = items[j + 1].text if j + 1 < len(items) else ""
                    if re.match(r"^\d{2,4}$", next_text) and re.match(r"^[A-Z]+\s+[A-Za-z]+", next_name):
                        break
                    if "Age Group" in next_text:
                        break

            if len(values) < 9:
                i += 1
                continue

            snatch1 = parse_attempt(values[2]["value"], values[2]["is_red"])
            snatch2 = parse_attempt(values[3]["value"], values[3]["is_red"])
            snatch3 = parse_attempt(values[4]["value"], values[4]["is_red"])
            cj1 = parse_attempt(values[5]["value"], values[5]["is_red"])
            cj2 = parse_attempt(values[6]["value"], values[6]["is_red"])
            cj3 = parse_attempt(values[7]["value"], values[7]["is_red"])

            results.append(
                {
                    "adaptive": adaptive,
                    "age": current_age_category,
                    "bodyWeight": float(values[0]["value"]),
                    "cj1": cj1,
                    "cj2": cj2,
                    "cj3": cj3,
                    "cjBest": best_positive([cj1, cj2, cj3]),
                    "date": date,
                    "eventId": event_id,
                    "federation": "USAMW",
                    "meet": meet,
                    "name": name,
                    "snatch1": snatch1,
                    "snatch2": snatch2,
                    "snatch3": snatch3,
                    "snatchBest": best_positive([snatch1, snatch2, snatch3]),
                    "total": int(float(values[8]["value"])),
                }
            )
            i = j

    return results


def scrape_request(request: dict[str, Any]) -> list[dict[str, Any]]:
    all_results: list[dict[str, Any]] = []
    for url in request["pdf_urls"]:
        print(f"Downloading {url}")
        pdf_bytes = fetch_bytes(url)
        pages = extract_pdf_text_items(pdf_bytes)
        rows = parse_results_from_pages(
            pages,
            request["meet"],
            request["date"],
            bool(request.get("adaptive", False)),
        )
        print(f"Parsed {len(rows)} row(s) from {url}")
        all_results.extend(rows)
    return all_results


def summarize_ingest(results: list[dict[str, Any]]) -> dict[str, int]:
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
        results = [pg.upsert_lifting_result(conn, row) for row in rows]
        conn.commit()
    return summarize_ingest(results)


def convex_action(endpoint: str, path_name: str, args: dict[str, Any]) -> dict[str, Any]:
    response = requests.post(
        endpoint,
        json={"path": path_name, "args": args},
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
        convex_action(endpoint, INGEST_PATH, {"scraperSecret": scraper_secret, **row})
        for row in rows
    ]
    return summarize_ingest(results)


def ingest_rows(rows: list[dict[str, Any]]) -> dict[str, dict[str, int]]:
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
    webhook_url = (
        os.getenv("SLACK_USAMW_RESULTS_WEBHOOK_URL")
        or os.getenv("SLACK_RESULTS_WEBHOOK_URL")
        or os.getenv("SLACK_WEBHOOK_URL")
    )
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


def process_request(request: dict[str, Any]) -> dict[str, Any]:
    rows = scrape_request(request)
    if not rows:
        raise RuntimeError(f"Parsed 0 lifting result rows for {request['meet']}")
    stats = ingest_rows(rows)
    message = f"USAMW results import complete for {request['meet']}\n{len(rows)} parsed row(s)\n{format_stats(stats)}"
    print(message)
    post_slack(message)
    return {"rows": len(rows), "stats": stats}


def request_dir() -> Path:
    if os.getenv("USAMW_RESULTS_REQUESTS_DIR"):
        return Path(os.environ["USAMW_RESULTS_REQUESTS_DIR"])
    if os.getenv("MEET_AUTOMATION_STATE_DIR"):
        return Path(os.environ["MEET_AUTOMATION_STATE_DIR"]) / "usamw_results_requests"
    return Path(__file__).resolve().parent / "state" / "usamw_results_requests"


def drain_requests() -> None:
    directory = request_dir()
    directory.mkdir(parents=True, exist_ok=True)
    done_dir = directory / "done"
    failed_dir = directory / "failed"
    done_dir.mkdir(exist_ok=True)
    failed_dir.mkdir(exist_ok=True)

    files = sorted(path for path in directory.iterdir() if path.suffix == ".json" and path.is_file())
    if not files:
        print(f"No USAMW results requests in {directory}")
        return

    for source in files:
        try:
            request = json.loads(source.read_text())
            process_request(request)
            shutil.move(str(source), done_dir / source.name)
        except Exception as exc:  # noqa: BLE001
            print(f"USAMW results request failed ({source.name}): {exc}", file=sys.stderr)
            post_slack(f"USAMW results import failed for {source.name}: {exc}")
            shutil.move(str(source), failed_dir / source.name)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Scrape USAMW PDF results into lifting_results")
    parser.add_argument("--requested", action="store_true", help="drain queued Slack requests")
    parser.add_argument("--meet")
    parser.add_argument("--date")
    parser.add_argument("--adaptive", action="store_true")
    parser.add_argument("--pdf", action="append", dest="pdf_urls", default=[])
    return parser


def main() -> int:
    args = build_parser().parse_args()
    if args.requested:
        drain_requests()
        return 0
    if not args.meet or not args.date or not args.pdf_urls:
        print(
            'Usage: python usamw_results.py --meet "Meet Name" --date YYYY-MM-DD --pdf https://... [--pdf https://...] [--adaptive]',
            file=sys.stderr,
        )
        return 2
    if not re.match(r"^\d{4}-\d{2}-\d{2}$", args.date):
        print("--date must be YYYY-MM-DD", file=sys.stderr)
        return 2

    process_request(
        {
            "meet": args.meet,
            "date": args.date,
            "adaptive": args.adaptive,
            "pdf_urls": args.pdf_urls,
            "requested_at_unix": int(time.time()),
        }
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
