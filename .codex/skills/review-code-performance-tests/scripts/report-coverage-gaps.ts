#!/usr/bin/env bun
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = process.cwd();
const LCOV_PATH = process.env.LCOV_PATH ?? join(ROOT, "coverage", "lcov.info");

const SOURCE_ROOTS = ["app/src", "scrapers/common", "scrapers/usaw/meet_automation"] as const;

const IGNORE_DIR_NAMES = new Set([
  "node_modules",
  "target",
  "coverage",
  "__pycache__",
  ".git",
  "tests",
  ".venv",
  ".runtime",
]);

type FileCoverage = {
  path: string;
  linesFound: number;
  linesHit: number;
};

function walkSourceFiles(dir: string, acc: string[]): void {
  let entries: string[] = [];
  try {
    entries = readdirSync(dir);
  } catch {
    return;
  }
  for (const entry of entries) {
    if (IGNORE_DIR_NAMES.has(entry)) continue;
    const full = join(dir, entry);
    let stats;
    try {
      stats = statSync(full);
    } catch {
      continue;
    }
    if (stats.isDirectory()) {
      walkSourceFiles(full, acc);
      continue;
    }
    if (!/\.(rs|py)$/.test(entry)) continue;
    if (/^(test_|conftest).*\.py$/.test(entry)) continue;
    if (/\.(test|spec)\.(rs|py)$/.test(entry)) continue;
    acc.push(full);
  }
}

function parseLcov(lcov: string): Map<string, FileCoverage> {
  const files = new Map<string, FileCoverage>();
  let current: FileCoverage | null = null;

  for (const line of lcov.split(/\r?\n/)) {
    if (line.startsWith("SF:")) {
      const raw = line.slice(3).trim();
      const path = raw.startsWith(ROOT)
        ? relative(ROOT, raw)
        : raw.replace(/^\.\//, "");
      current = { path, linesFound: 0, linesHit: 0 };
    } else if (line.startsWith("DA:") && current) {
      current.linesFound += 1;
      const hit = Number(line.split(",")[1] ?? "0");
      if (hit > 0) current.linesHit += 1;
    } else if (line.startsWith("end_of_record") && current) {
      files.set(current.path, current);
      current = null;
    }
  }
  return files;
}

function percent(hit: number, found: number): number {
  if (found <= 0) return 0;
  return (100 * hit) / found;
}

function relatedTestHint(file: string): string {
  const candidates: string[] = [];
  if (file.endsWith(".rs")) {
    const withoutExt = file.replace(/\.rs$/, "");
    candidates.push(`${withoutExt}.rs`);
    const route = file.match(/^app\/src\/routes\/([^/]+)\//);
    if (route) candidates.push(`app/tests/${route[1]}.rs`);
    if (file.startsWith("app/src/")) candidates.push("app/tests/support/mod.rs");
  }
  if (file.endsWith(".py")) {
    const dir = file.replace(/\/[^/]+$/, "");
    const base = file.replace(/^.*\//, "").replace(/\.py$/, "");
    candidates.push(`${dir}/tests/test_${base}.py`);
    candidates.push(`${dir}/test_${base}.py`);
    candidates.push(`scrapers/common/tests/test_${base}.py`);
    candidates.push(`scrapers/usaw/meet_automation/tests/test_meet_automation.py`);
    if (base === "postgres_writer") candidates.push("scrapers/common/test_postgres_writer.py");
    if (base === "postgres_ingest") candidates.push("scrapers/common/tests/test_postgres_ingest.py");
    if (base === "ingest") candidates.push("scrapers/common/tests/test_postgres_ingest.py");
  }

  const found = [
    ...new Set(
      candidates.filter((candidate) => candidate !== file && existsSync(join(ROOT, candidate))),
    ),
  ];
  if (file.endsWith(".rs") && existsSync(join(ROOT, file))) {
    try {
      const src = readFileSync(join(ROOT, file), "utf8");
      if (src.includes("#[cfg(test)]")) found.push("(inline #[cfg(test)])");
    } catch {
      // ignore unreadable source
    }
  }
  return found.length > 0 ? found.join(", ") : "(none next to source)";
}

function main(): void {
  const coverage = existsSync(LCOV_PATH)
    ? parseLcov(readFileSync(LCOV_PATH, "utf8"))
    : new Map<string, FileCoverage>();

  const sources: string[] = [];
  for (const root of SOURCE_ROOTS) {
    walkSourceFiles(join(ROOT, root), sources);
  }

  const rows = sources
    .map((full) => relative(ROOT, full))
    .map((path) => {
      const entry = coverage.get(path);
      const found = entry?.linesFound ?? 0;
      const hit = entry?.linesHit ?? 0;
      return {
        path,
        found,
        hit,
        pct: percent(hit, found),
        missingFromLcov: !entry,
        testHint: relatedTestHint(path),
      };
    })
    .sort((a, b) => a.pct - b.pct || a.path.localeCompare(b.path));

  const uncovered = rows.filter(
    (row) => row.missingFromLcov || row.pct === 0,
  );
  const low = rows.filter((row) => !row.missingFromLcov && row.pct > 0 && row.pct < 50);
  const noAdjacentTest = rows.filter((row) => row.testHint === "(none next to source)");

  console.log(`lcov: ${existsSync(LCOV_PATH) ? LCOV_PATH : "(none — inventory mode)"}`);
  console.log(`source files: ${rows.length}`);
  console.log(`zero coverage or missing from lcov: ${uncovered.length}`);
  console.log(`hit but <50%: ${low.length}`);
  console.log(`no adjacent/integration test hint: ${noAdjacentTest.length}`);
  console.log("");
  console.log("Risk-first gaps (not a vanity % target):");
  console.log("");

  const print = (title: string, items: typeof rows, limit: number) => {
    console.log(`## ${title}`);
    if (items.length === 0) {
      console.log("(none)");
      console.log("");
      return;
    }
    for (const row of items.slice(0, limit)) {
      const pctLabel = row.missingFromLcov
        ? "missing"
        : `${row.pct.toFixed(1)}% (${row.hit}/${row.found})`;
      console.log(`- ${row.path}  ${pctLabel}  tests: ${row.testHint}`);
    }
    if (items.length > limit) {
      console.log(`- … ${items.length - limit} more`);
    }
    console.log("");
  };

  print("No adjacent test", noAdjacentTest, 40);
  if (existsSync(LCOV_PATH)) {
    print("Zero / missing lcov", uncovered, 40);
    print("Below 50%", low, 20);
  }
}

main();
