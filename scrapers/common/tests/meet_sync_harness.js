// Preloaded (`node --require meet_sync_harness.js <sync script>`) by
// test_meet_sync_batching.py so the meet-sync scripts run offline and without
// their node_modules:
//
// - `axios.get` answers with the Sport80 widget payload in $MEET_SYNC_FIXTURE
//   (a JSON array of raw meets) instead of calling the API.
// - `axios.post` (the Slack webhook) appends the posted body, as one JSON line,
//   to $MEET_SYNC_SLACK_LOG.
// - `dotenv.config()` does nothing, so no local .env leaks into the test.
//
// The Postgres side is faked separately, through $POSTGRES_INGEST_PYTHON.

const fs = require('fs');
const Module = require('module');

const fakeAxios = {
  async get() {
    const meets = JSON.parse(fs.readFileSync(process.env.MEET_SYNC_FIXTURE, 'utf8'));
    return { status: 200, data: { data: meets } };
  },
  async post(url, body) {
    fs.appendFileSync(process.env.MEET_SYNC_SLACK_LOG, `${JSON.stringify({ url, body })}\n`);
    return { status: 200, data: 'ok' };
  },
};

const fakeDotenv = { config: () => ({ parsed: {} }) };

const originalLoad = Module._load;
Module._load = function load(request, parent, isMain) {
  if (request === 'axios') return fakeAxios;
  if (request === 'dotenv') return fakeDotenv;
  return originalLoad.call(this, request, parent, isMain);
};
