// Prints, as a JSON array, `placeholderMemberId(name)` for every case in the
// shared fixture. Run by test_placeholder_parity.py, which compares the output
// with the fixture's `expected` values and with the Python rule.
//
//   node placeholder_parity.js fixtures/placeholder_member_ids.json

const fs = require('fs');
const path = require('path');
const { placeholderMemberId } = require(
    path.join(__dirname, '..', '..', 'usaw', 'entry_scraper', 'placeholder_member_id.js')
);

const fixturePath = process.argv[2] || path.join(__dirname, 'fixtures', 'placeholder_member_ids.json');
const { cases } = JSON.parse(fs.readFileSync(fixturePath, 'utf8'));
process.stdout.write(JSON.stringify(cases.map((c) => placeholderMemberId(c.name))));
