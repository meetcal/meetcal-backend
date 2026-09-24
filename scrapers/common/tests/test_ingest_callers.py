"""Scrapers write through ``IngestClient.actions`` / ``actions_skipping_errors``.

No database: each scraper's write step runs against a fake client that
records the calls, so these check the shape of the write (one batch call, not
one ``action`` per row) and that each scraper kept its old per-row error
semantics. Third-party modules the scrapers import only for scraping (HTTP,
PDF, HTML, dotenv, the Sport80 client) are stubbed while the module is
imported, since CI installs only ``requirements.txt``.

A static check also walks every scraper source for ``.action(`` inside a loop.
"""

from __future__ import annotations

import ast
import importlib
import io
import sys
import types
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

try:
    import psycopg  # noqa: F401 - postgres_ingest imports it

    from common.postgres_ingest import RowFailure
except ImportError:  # pragma: no cover - optional local dep
    RowFailure = None

SCRAPERS_DIR = Path(__file__).resolve().parents[2]


def _stub_modules() -> dict[str, types.ModuleType]:
    dotenv = types.ModuleType("dotenv")
    dotenv.load_dotenv = lambda *args, **kwargs: False
    sport80 = types.ModuleType("sport80")
    sport80.SportEighty = object
    bs4 = types.ModuleType("bs4")
    bs4.BeautifulSoup = object
    pandas = types.ModuleType("pandas")
    pandas.DataFrame = object  # used in annotations at import time
    requests = types.ModuleType("requests")
    requests.RequestException = type("RequestException", (Exception,), {})
    requests.exceptions = types.SimpleNamespace(
        RequestException=requests.RequestException
    )
    return {
        "requests": requests,
        "dotenv": dotenv,
        "pdfplumber": types.ModuleType("pdfplumber"),
        "pandas": pandas,
        "bs4": bs4,
        "sport80": sport80,
    }


def _import_scraper(module: str) -> types.ModuleType:
    """Import a scraper module with its scraping-only deps stubbed.

    ``patch.dict`` restores ``sys.modules`` afterwards, so neither the stubs
    nor the scraper module leak into other tests.
    """
    with patch.dict(sys.modules, _stub_modules()):
        sys.modules.pop(module, None)
        return importlib.import_module(module)


class FakeIngestClient:
    def __init__(self, results=None, raises: Exception | None = None) -> None:
        self.results = results
        self.raises = raises
        self.calls: list[tuple[str, str, list[dict]]] = []

    def _record(self, method: str, path: str, rows) -> list:
        rows = list(rows)
        self.calls.append((method, path, rows))
        if self.raises is not None:
            raise self.raises
        if self.results is not None:
            return self.results
        return [{"wasInsert": True} for _ in rows]

    def action(self, path, args):  # pragma: no cover - must not be called
        raise AssertionError("per-row action() called")

    def actions(self, path, rows):
        return self._record("actions", path, rows)

    def actions_skipping_errors(self, path, rows):
        return self._record("actions_skipping_errors", path, rows)


@unittest.skipUnless(RowFailure is not None, "psycopg is required to import postgres_ingest")
class BatchedIngestCallerTests(unittest.TestCase):
    def test_bwl_results_skip_bad_rows_in_one_batch(self) -> None:
        module = _import_scraper("bwl.sport80_api.update_supabase_from_sport80")
        rows = [{"name": "A", "meet": "M"}, {"name": "B", "meet": "M"}, {"name": "C", "meet": "M"}]
        client = FakeIngestClient(
            results=[{"wasInsert": True}, RowFailure(1, ValueError("bad")), {"wasInsert": False}]
        )
        with self.assertLogs(level="INFO") as logs:
            module.add_meet_results_to_postgres(client, rows)
        self.assertEqual(
            client.calls,
            [("actions_skipping_errors", "scraperIngestion:ingestLiftingResult", rows)],
        )
        output = "\n".join(logs.output)
        self.assertIn("Error upserting result for 'B' in Postgres: bad", output)
        self.assertIn("Successfully upserted 2/3 results via Postgres.", output)

    def test_bwl_results_log_rather_than_raise_on_a_connection_failure(self) -> None:
        module = _import_scraper("bwl.sport80_api.update_supabase_from_sport80")
        client = FakeIngestClient(raises=RuntimeError("no connection"))
        with self.assertLogs(level="INFO") as logs:
            module.add_meet_results_to_postgres(client, [{"name": "A", "meet": "M"}])
        self.assertIn("Successfully upserted 0/1", "\n".join(logs.output))

    def test_standards_are_one_all_or_nothing_batch(self) -> None:
        module = _import_scraper("usaw.standards_scraper.scraper")
        scraper = module.StandardsScraper.__new__(module.StandardsScraper)
        standards = [
            {"age_category": "Senior", "gender": "M", "weight_class": "71", "standard_a": 300, "standard_b": 280},
            {"age_category": "Senior", "gender": "F", "weight_class": "59", "standard_a": 200, "standard_b": 190},
        ]
        scraper.ingest = FakeIngestClient(results=[{"wasInsert": True}, {"wasChanged": True}])
        scraper.scraper_secret = "s"
        with redirect_stdout(io.StringIO()):
            result = scraper.upsert_to_postgres(standards)
        self.assertEqual(result, {"inserted": [standards[0]], "updated": [standards[1]]})
        [(method, path, rows)] = scraper.ingest.calls
        self.assertEqual((method, path, len(rows)), ("actions", "scraperIngestion:ingestStandard", 2))

        # A failing row still raises, as the per-row loop did.
        scraper.ingest = FakeIngestClient(raises=RuntimeError("boom"))
        with self.assertRaises(RuntimeError), redirect_stdout(io.StringIO()):
            scraper.upsert_to_postgres(standards)

    def test_usamw_qualifying_totals_are_one_all_or_nothing_batch(self) -> None:
        module = _import_scraper("usamw.qt.scraper_qt")
        scraper = module.USAMWQTScraper.__new__(module.USAMWQTScraper)
        records = [
            {"event_name": "E", "gender": "M", "age_category": "M35", "weight_class": "71", "qualifying_total": "200"},
            {"event_name": "E", "gender": "F", "age_category": "W35", "weight_class": "59", "qualifying_total": "120"},
        ]
        scraper.ingest = FakeIngestClient(results=[{"wasInsert": True}, {"wasInsert": False}])
        scraper.scraper_secret = "s"
        with redirect_stdout(io.StringIO()):
            result = scraper.upsert_to_postgres(records)
        self.assertEqual(result, {"inserted": [records[0]], "updated": [records[1]]})
        [(method, path, rows)] = scraper.ingest.calls
        self.assertEqual(method, "actions")
        self.assertEqual(path, "scraperIngestion:ingestQualifyingTotal")
        self.assertEqual([row["qualifyingTotal"] for row in rows], [200, 120])

        scraper.ingest = FakeIngestClient(raises=RuntimeError("boom"))
        with self.assertRaises(RuntimeError), redirect_stdout(io.StringIO()):
            scraper.upsert_to_postgres(records)

    def test_usamw_meets_skip_bad_events_in_one_batch(self) -> None:
        module = _import_scraper("usamw.meets.scrape_events")
        scraper = module.USAMWEventsScraper.__new__(module.USAMWEventsScraper)

        def event(name: str) -> dict:
            return {
                "name": name,
                "venue_name": "V",
                "venue_street": "S",
                "venue_city": "C",
                "venue_state": "ST",
                "venue_zip": "Z",
                "time_zone": "America/New_York",
                "start_date": "2026-10-01",
                "end_date": "2026-10-02",
                "status": "upcoming",
                "federation": "USAMW",
            }

        missing_field = event("Missing")
        del missing_field["venue_zip"]
        events = [event("New"), missing_field, event("Existing"), event("Bad")]
        scraper.ingest = FakeIngestClient(
            results=[{"wasInsert": True}, {"wasInsert": False}, RowFailure(2, ValueError("bad row"))]
        )
        scraper.scraper_secret = "s"
        out = io.StringIO()
        with redirect_stdout(out):
            result = scraper.ingest_to_postgres(events)
        self.assertEqual(result, {"inserted": [events[0]], "skipped": [events[2]]})
        [(method, path, rows)] = scraper.ingest.calls
        self.assertEqual((method, path), ("actions_skipping_errors", "scraperIngestion:ingestMeet"))
        self.assertEqual([row["name"] for row in rows], ["New", "Existing", "Bad"])
        self.assertIn("Error ingesting Missing: 'venue_zip'", out.getvalue())
        self.assertIn("Error ingesting Bad: bad row", out.getvalue())

        # A connection failure is reported per event, never raised.
        scraper.ingest = FakeIngestClient(raises=RuntimeError("no connection"))
        out = io.StringIO()
        with redirect_stdout(out):
            result = scraper.ingest_to_postgres([event("New")])
        self.assertEqual(result, {"inserted": [], "skipped": []})
        self.assertIn("Error ingesting New: no connection", out.getvalue())


class NoPerRowActionTests(unittest.TestCase):
    """``IngestClient.action`` must not be called from inside a loop."""

    EXCLUDED_PARTS = {"urlwatch", "tests", "node_modules", ".venv", "__pycache__"}

    def test_no_scraper_calls_action_inside_a_loop(self) -> None:
        offenders = []
        for path in sorted(SCRAPERS_DIR.rglob("*.py")):
            relative = path.relative_to(SCRAPERS_DIR)
            if self.EXCLUDED_PARTS.intersection(relative.parts) or path.name.startswith("test_"):
                continue
            try:
                tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
            except SyntaxError:
                continue
            for loop in ast.walk(tree):
                if not isinstance(loop, (ast.For, ast.AsyncFor, ast.While, ast.comprehension)):
                    continue
                for node in ast.walk(loop if not isinstance(loop, ast.comprehension) else loop.iter):
                    if (
                        isinstance(node, ast.Call)
                        and isinstance(node.func, ast.Attribute)
                        and node.func.attr == "action"
                    ):
                        offenders.append(f"{relative}:{node.lineno}")
        self.assertEqual(
            sorted(set(offenders)),
            [],
            "use IngestClient.actions / actions_skipping_errors instead of action() in a loop",
        )


if __name__ == "__main__":
    unittest.main()
