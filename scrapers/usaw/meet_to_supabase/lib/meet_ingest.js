// Batched Postgres writes for the meet-sync scripts (sync-meets.js,
// sync-nat-meets.js, sync-virus-meets.js).
//
// Each run hands all of its meets to `scrapers/common/postgres_ingest.py` in
// one process (one connection, one transaction) instead of one process per
// meet. The ingest CLI refuses stdin past MAX_STDIN_BYTES (16 MiB) or
// MAX_STDIN_ROWS (20,000) rows, so a run is split into chunks well under both
// caps; a normal run (a page of ~20 meets) is a single chunk.
//
// Meets are independent upserts, so writes use `--skip-errors`: each meet gets
// its own savepoint and a bad meet is reported and skipped without costing the
// rest, which is what the old per-meet loop did.

const path = require('path');
const { spawnSync } = require('child_process');

const COMMON_DIR = path.resolve(__dirname, '../../../common');
const INGEST_SCRIPT = path.join(COMMON_DIR, 'postgres_ingest.py');
const LOOKUP_SCRIPT = path.join(COMMON_DIR, 'lookup_meet_status.py');

// Mirrors of the ingest CLI's stdin caps (common/postgres_ingest.py). A test
// checks they match the Python constants.
const INGEST_STDIN_MAX_BYTES = 16 * 1024 * 1024;
const INGEST_STDIN_MAX_ROWS = 20000;

// Per-call chunk limits: a quarter of the byte cap and a twentieth of the row
// cap, so a chunk never gets near a refusal even with JSON framing overhead.
const INGEST_CHUNK_MAX_ROWS = 1000;
const INGEST_CHUNK_MAX_BYTES = 4 * 1024 * 1024;

// Bounds on one child process: its stdout (one small JSON result per row) and
// its run time, so a hung connection cannot hold the cron lock forever.
const INGEST_MAX_OUTPUT_BYTES = 16 * 1024 * 1024;
const INGEST_TIMEOUT_MS = 10 * 60 * 1000;

if (INGEST_CHUNK_MAX_ROWS > INGEST_STDIN_MAX_ROWS || INGEST_CHUNK_MAX_BYTES > INGEST_STDIN_MAX_BYTES) {
  throw new Error('meet_ingest chunk limits exceed the ingest stdin caps');
}

const MEET_INGEST_PATH = 'scraperIngestion:ingestMeet';

function pythonCommand() {
  return process.env.POSTGRES_INGEST_PYTHON || 'python3';
}

// Split `items` into consecutive chunks of at most `maxRows` items whose JSON
// array encoding is at most `maxBytes` bytes. An item that is too big on its
// own gets a chunk of its own; the CLI then refuses it (exit 3) and only that
// chunk's meets are reported as failed.
function chunkForIngest(items, maxRows = INGEST_CHUNK_MAX_ROWS, maxBytes = INGEST_CHUNK_MAX_BYTES) {
  if (!Number.isInteger(maxRows) || maxRows < 1 || !Number.isInteger(maxBytes) || maxBytes < 2) {
    throw new Error(`invalid chunk limits: rows=${maxRows} bytes=${maxBytes}`);
  }
  const chunks = [];
  let current = [];
  let currentBytes = 2; // "[" and "]"
  for (const item of items) {
    const itemBytes = Buffer.byteLength(JSON.stringify(item), 'utf8');
    const separator = current.length > 0 ? 1 : 0; // ","
    if (current.length > 0 && (current.length >= maxRows || currentBytes + separator + itemBytes > maxBytes)) {
      chunks.push(current);
      current = [];
      currentBytes = 2;
    }
    currentBytes += (current.length > 0 ? 1 : 0) + itemBytes;
    current.push(item);
  }
  if (current.length > 0) chunks.push(current);
  return chunks;
}

function describeFailure(label, result) {
  if (result.error) return `${label} could not run: ${result.error.message}`;
  if (result.signal) return `${label} was killed by ${result.signal}`;
  const detail = String(result.stderr || result.stdout || '').trim();
  return `${label} exited with code ${result.status}${detail ? `: ${detail}` : ''}`;
}

function runPython(script, args, payload, label) {
  const result = spawnSync(pythonCommand(), [script, ...args], {
    input: JSON.stringify(payload),
    encoding: 'utf8',
    env: process.env,
    maxBuffer: INGEST_MAX_OUTPUT_BYTES,
    timeout: INGEST_TIMEOUT_MS,
  });
  if (result.error || result.status !== 0) {
    throw new Error(describeFailure(label, result));
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`${label} printed output that is not JSON: ${String(result.stdout).slice(0, 200)}`);
  }
}

function meetIngestRow(meet) {
  return {
    name: meet.name,
    venueName: meet.venueName,
    venueStreet: meet.venueStreet,
    venueCity: meet.venueCity,
    venueState: meet.venueState,
    venueZip: meet.venueZip,
    timeZone: meet.timeZone,
    startDate: meet.startDate,
    endDate: meet.endDate,
    status: meet.status,
    federation: meet.federation,
  };
}

// Upsert every meet, one ingest process per chunk. Returns one outcome per
// meet, in order: `{ meet, ok: true, wasInsert }` or `{ meet, ok: false, error }`,
// plus `chunkFailures`, the number of whole ingest calls that failed (process
// error, non-zero exit, unreadable output). A skipped meet is a per-meet
// failure; a failed call fails every meet in that chunk and is also counted
// in `chunkFailures` so the caller can exit non-zero.
function ingestMeets(meets) {
  const outcomes = [];
  let chunkFailures = 0;
  const chunks = chunkForIngest(meets.map(meetIngestRow));
  let offset = 0;
  chunks.forEach((rows, chunkIndex) => {
    const chunkMeets = meets.slice(offset, offset + rows.length);
    offset += rows.length;
    const label = `Postgres ingest (chunk ${chunkIndex + 1}/${chunks.length}, ${rows.length} meets)`;
    let results;
    try {
      results = runPython(INGEST_SCRIPT, ['--skip-errors', MEET_INGEST_PATH], rows, label);
      if (!Array.isArray(results) || results.length !== rows.length) {
        throw new Error(`${label} returned ${Array.isArray(results) ? results.length : 'no'} results for ${rows.length} meets`);
      }
    } catch (error) {
      chunkFailures++;
      console.error(error.message);
      for (const meet of chunkMeets) outcomes.push({ meet, ok: false, error: `not written; ${label} failed` });
      return;
    }
    results.forEach((result, i) => {
      const meet = chunkMeets[i];
      if (result && Object.prototype.hasOwnProperty.call(result, 'rowError')) {
        outcomes.push({ meet, ok: false, error: String(result.rowError) });
      } else {
        outcomes.push({ meet, ok: true, wasInsert: Boolean(result && result.wasInsert) });
      }
    });
  });
  return { outcomes, chunkFailures };
}

// Existing status per meet name (null when unknown or when the lookup
// failed), one lookup process per chunk. A failed lookup is logged and its
// names are treated as unknown, as the old per-meet lookup did.
function lookupMeetStatuses(names) {
  const statuses = new Map();
  if (!process.env.DATABASE_URL) return statuses;
  const chunks = chunkForIngest(names);
  chunks.forEach((chunk, chunkIndex) => {
    const label = `Status lookup (chunk ${chunkIndex + 1}/${chunks.length}, ${chunk.length} meets)`;
    try {
      const parsed = runPython(LOOKUP_SCRIPT, [], { names: chunk }, label);
      const found = (parsed && parsed.statuses) || {};
      for (const name of chunk) {
        statuses.set(name, Object.prototype.hasOwnProperty.call(found, name) ? found[name] || null : null);
      }
    } catch (error) {
      console.error(error.message);
    }
  });
  return statuses;
}

module.exports = {
  INGEST_STDIN_MAX_BYTES,
  INGEST_STDIN_MAX_ROWS,
  INGEST_CHUNK_MAX_ROWS,
  INGEST_CHUNK_MAX_BYTES,
  chunkForIngest,
  ingestMeets,
  lookupMeetStatuses,
  meetIngestRow,
};
