"""Scrapers write through ``IngestClient.actions`` / ``actions_skipping_errors``.

No database: each scraper's write step runs against a fake client that
records the calls, so these check the shape of the write (one batch call, not
one ``action`` per row) and that each scraper kept its old per-row error
semantics. Third-party modules the scrapers import only for scraping (HTTP,
PDF, HTML, dotenv, the Sport80 client) are stubbed while the module is
imported, since CI installs only ``requirements.txt``.

A static check also walks every scraper source for ``.action(`` inside a loop,
and every JS scraper for a child process (the ingest CLI) started inside a
loop, which is the JS spelling of the same per-row pattern.
"""

from __future__ import annotations

import ast
import importlib
import io
import re
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


# --- JS: no child process per row -------------------------------------------

_JS_SPAWN = re.compile(r"\b(?:spawnSync|spawn|execFileSync|execFile|execSync|exec|fork)\s*\(")
_JS_FUNCTION = re.compile(
    r"\bfunction\s+([A-Za-z_$][\w$]*)\s*\("
    r"|\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:function\b|\([^()]*\)\s*=>|[A-Za-z_$][\w$]*\s*=>)"
)
_JS_FOR_WHILE = re.compile(r"\b(?:for|while)\s*(?:await\s*)?\(")
_JS_ITERATOR = re.compile(
    r"([A-Za-z_$][\w$.]*(?:\([^()]*\))?)\s*\.\s*(?:forEach|map|flatMap|reduce|filter|some|every|find)\s*\("
)
_JS_REGEX_PREFIX = set("(,=:[!&|?{};+-*%<>~^")


def _js_strip(source: str) -> str:
    """Blank out JS comments, strings, template literals and regex literals.

    Keeps offsets and newlines so line numbers survive. A heuristic lexer, good
    enough for the scrapers' plain Node scripts; it only has to keep braces and
    parentheses inside literals from confusing the block matcher below.
    """
    out = list(source)
    i, n = 0, len(source)
    last_significant = ""

    def blank(start: int, end: int) -> None:
        for k in range(start, min(end, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        ch = source[i]
        nxt = source[i + 1] if i + 1 < n else ""
        if ch == "/" and nxt == "/":
            end = source.find("\n", i)
            end = n if end == -1 else end
            blank(i, end)
            i = end
            continue
        if ch == "/" and nxt == "*":
            end = source.find("*/", i + 2)
            end = n if end == -1 else end + 2
            blank(i, end)
            i = end
            continue
        if ch in "'\"`":
            j = i + 1
            while j < n and source[j] != ch:
                j += 2 if source[j] == "\\" else 1
            blank(i + 1, j)
            i = j + 1
            last_significant = ch
            continue
        if ch == "/" and (last_significant == "" or last_significant in _JS_REGEX_PREFIX):
            j, in_class = i + 1, False
            while j < n and source[j] != "\n":
                c = source[j]
                if c == "\\":
                    j += 2
                    continue
                if c == "[":
                    in_class = True
                elif c == "]":
                    in_class = False
                elif c == "/" and not in_class:
                    break
                j += 1
            blank(i + 1, j)
            i = j + 1
            last_significant = "/"
            continue
        if not ch.isspace():
            last_significant = ch
        i += 1
    return "".join(out)


def _js_match(text: str, open_index: int) -> int:
    """Index just past the bracket that closes ``text[open_index]``."""
    pairs = {"(": ")", "{": "}", "[": "]"}
    closer = pairs[text[open_index]]
    depth = 0
    for k in range(open_index, len(text)):
        if text[k] == text[open_index]:
            depth += 1
        elif text[k] == closer:
            depth -= 1
            if depth == 0:
                return k + 1
    return len(text)


def _js_function_bodies(text: str) -> dict[str, str]:
    bodies: dict[str, str] = {}
    for match in _JS_FUNCTION.finditer(text):
        name = match.group(1) or match.group(2)
        brace = text.find("{", match.end() - 1)
        if brace == -1:
            continue
        # An arrow with an expression body has no brace before the next ";".
        semicolon = text.find(";", match.end())
        if semicolon != -1 and semicolon < brace and match.group(2):
            bodies[name] = text[match.end():semicolon]
            continue
        bodies[name] = text[brace:_js_match(text, brace)]
    return bodies


def _js_loops(text: str):
    """Yield ``(header, body, offset)`` for each ``for``/``while`` loop and
    array-iterator callback (``xs.forEach(...)``, ``xs.map(...)``, ...)."""
    for match in _JS_FOR_WHILE.finditer(text):
        paren = match.end() - 1
        header_end = _js_match(text, paren)
        rest = text[header_end:]
        stripped = rest.lstrip()
        body_start = header_end + (len(rest) - len(stripped))
        if stripped.startswith("{"):
            body_end = _js_match(text, body_start)
        else:
            body_end = text.find(";", body_start)
            body_end = len(text) if body_end == -1 else body_end
        yield text[paren:header_end], text[body_start:body_end], match.start()
    for match in _JS_ITERATOR.finditer(text):
        paren = match.end() - 1
        yield match.group(1), text[paren:_js_match(text, paren)], match.start()


def js_spawns_in_loops(sources: dict[str, str]) -> list[str]:
    """``file:line`` of every loop that starts a child process per iteration.

    A function that spawns (directly, or by calling one that does, in any of
    ``sources``) counts as a spawn. The one exempt shape is a loop over
    ingest chunks (its header names a ``chunk``): a bounded number of batched
    calls, each well under the ingest CLI's stdin caps.
    """
    stripped = {name: _js_strip(text) for name, text in sources.items()}
    bodies: dict[str, str] = {}
    for text in stripped.values():
        bodies.update(_js_function_bodies(text))
    spawners = {name for name, body in bodies.items() if _JS_SPAWN.search(body)}
    while True:
        calls = re.compile(r"\b(?:" + "|".join(map(re.escape, sorted(spawners))) + r")\s*\(") if spawners else None
        grown = {
            name for name, body in bodies.items()
            if name not in spawners and calls is not None and calls.search(body)
        }
        if not grown:
            break
        spawners |= grown
    calls = re.compile(r"\b(?:" + "|".join(map(re.escape, sorted(spawners))) + r")\s*\(") if spawners else None

    offenders = []
    for name, text in stripped.items():
        for header, body, offset in _js_loops(text):
            if re.search(r"chunk", header, re.I):
                continue
            if _JS_SPAWN.search(body) or (calls is not None and calls.search(body)):
                offenders.append(f"{name}:{text.count(chr(10), 0, offset) + 1}")
    return sorted(set(offenders))


class NoPerRowJsSpawnTests(unittest.TestCase):
    """JS scrapers must not start an ingest process per meet / row."""

    EXCLUDED_PARTS = NoPerRowActionTests.EXCLUDED_PARTS

    def test_the_check_flags_the_old_per_meet_spawn(self) -> None:
        per_meet = """
            const { spawnSync } = require('child_process');
            function ingestMeetToPostgres(meet) {
              const result = spawnSync(python, [ingestScript, 'x'], { input: JSON.stringify(meet) });
              return JSON.parse(result.stdout); // '{' in a comment
            }
            async function ingestMeet(meet) { return ingestMeetToPostgres(meet); }
            async function syncMeets(meets) {
              const re = /[{(]/g;
              for (const meet of meets) {
                const label = `meet ${meet.name} }`;
                await ingestMeet(meet);
              }
            }
        """
        self.assertEqual(js_spawns_in_loops({"old.js": per_meet}), ["old.js:10"])
        each = "const run = (row) => spawnSync('python3', [row]);\nrows.forEach(row => { run(row); });\n"
        self.assertEqual(js_spawns_in_loops({"each.js": each}), ["each.js:2"])

    def test_the_check_allows_one_call_per_chunk(self) -> None:
        batched = """
            function runPython(rows) { return spawnSync('python3', ['ingest.py'], { input: JSON.stringify(rows) }); }
            function ingestAll(rows) {
              const chunks = chunkForIngest(rows);
              chunks.forEach((chunk) => runPython(chunk));
              for (const chunk of chunks) runPython(chunk);
              for (const row of rows) console.log(row.name);
            }
            ingestAll(rows);
        """
        self.assertEqual(js_spawns_in_loops({"new.js": batched}), [])

    def test_no_js_scraper_spawns_inside_a_loop(self) -> None:
        sources = {}
        for path in sorted(SCRAPERS_DIR.rglob("*.js")):
            relative = path.relative_to(SCRAPERS_DIR)
            if self.EXCLUDED_PARTS.intersection(relative.parts):
                continue
            sources[str(relative)] = path.read_text(encoding="utf-8")
        self.assertIn("usaw/meet_to_supabase/scripts/sync-meets.js", sources)
        self.assertEqual(
            js_spawns_in_loops(sources),
            [],
            "batch rows into one ingest call per run (chunked under the stdin caps), "
            "not one child process per row",
        )

if __name__ == "__main__":
    unittest.main()
