// Deterministic stand-in for a missing membership number. Mirrors
// `placeholder_member_id` / `normalize_name` in scrapers/common/normalize.py:
// the writer keys an athlete carrying a `noid:` id on (meet, normalized name),
// so every nightly re-scrape updates the same row instead of minting a new one.
//
// Kept in its own dependency-free module so the parity test
// (scrapers/common/tests/test_placeholder_parity.py, fixture
// scrapers/common/tests/fixtures/placeholder_member_ids.json) can load it
// without launching the scraper.

const MEMBER_ID_PLACEHOLDER_PREFIX = 'noid:';

function placeholderMemberId(name) {
    // `str(value)` in Python: only null/undefined become '' (0 is "0").
    const text = name === null || name === undefined ? '' : String(name);
    const normalized = text.split(/\s+/).filter(Boolean).join(' ').toLowerCase();
    const slug = normalized.replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
    return `${MEMBER_ID_PLACEHOLDER_PREFIX}${slug}`;
}

module.exports = { MEMBER_ID_PLACEHOLDER_PREFIX, placeholderMemberId };
