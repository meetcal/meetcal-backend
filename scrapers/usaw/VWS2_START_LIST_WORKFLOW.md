# VWS2 / American Open 2 Final Start List Workflow

End-to-end process used for **2026 Virus Weightlifting Series 2, Powered by Rogue Fitness** (VWS2 / American Open 2): scrape the WSO-table start list PDF, verify against the schedule, then replace meet athletes in Postgres.

## Meet And Sources

| Item | Value |
| --- | --- |
| Meet name (must match exactly) | `2026 Virus Weightlifting Series 2, Powered by Rogue Fitness` |
| Start list PDF | [2026 VWS2 Start List](https://assets.contentstack.io/v3/assets/blteb7d012fc7ebef7f/bltf77c5ae5086fcf79/6a972c04e1500a3542016f2a/2026_-_VWS2_-_Start_List.pdf) |
| Source format | `wso_table` (new parser — not masters / owlcms / registration) |
| Scraped output | `scrapers/usaw/owlcms_schedule_scraper/vws2_final_start_list.ts` |
| Verification report | `scrapers/usaw/owlcms_schedule_scraper/vws2_final_start_list_report.txt` |
| Schedule (coverage checks) | `scrapers/usaw/owlcms_schedule_scraper/vws2_schedule_preview.ts` |

## PDF Shape: WSO Table

Header columns:

`WSO | Lot | First Name | Last Name | Nationality | Year | Age | Club Name | COMPETITIONS | Entry | Group | Sess. | Plat | Day | Time`

Parser module: `scrapers/usaw/final_start_scraper/wso_table_scraper.py`

Detection and extraction are wired in `scrapers/usaw/final_start_scraper/scraper.py` as source format `wso_table`.

Notable parsing rules for this meet:

- **`memberId` = Lot** from the PDF (intentional; do not run sequential `assign_member_ids` over these rows).
- **Weight class** comes from `COMPETITIONS`, preferring `OPEN W/M`, youth, adaptive, and university tokens over masters-age bleed (`W45 69` vs `OPEN W 69`).
- **Superheavy classes** normalize to trailing `+` (`86+`, `110+`), not prefix `+86`.
- **Club bleed** — strip trailing `OP`, `OPEN`, `OPE`, `N` when competition text leaked into the club cell.
- **Entry totals** — many cells are mangled OCR, not plain integers. The parser handles patterns including:
  - plain totals and youth totals down to **15**
  - `861+N /M` bleed from 86+ weight class
  - `1101 x/y` and `1102 x/y` slash forms
  - `MIL 1M4 07`-style WSO entry cells
  - `M1IL4 W8`, `MI2L0 M0`, explicit `M26IL1`
  - fused `111xx` / `112xx` weight-class bleed in the entry column
- **Withdrawn athletes** — rows with `WWW` session/platform and entry `0` are skipped (10 rows on this PDF).
- **Expected athlete count** — **677** parsed rows (687 PDF lots minus 10 withdrawn).

Known schedule gap (not a scrape bug): **session 18 / Red** has no athletes in the PDF.

## 1. Scrape

From `scrapers/usaw/final_start_scraper` using the project venv:

```bash
cd scrapers/usaw/final_start_scraper
venv/bin/python scraper.py \
  --pdf-url "https://assets.contentstack.io/v3/assets/blteb7d012fc7ebef7f/bltf77c5ae5086fcf79/6a972c04e1500a3542016f2a/2026_-_VWS2_-_Start_List.pdf" \
  --meet-name "2026 Virus Weightlifting Series 2, Powered by Rogue Fitness" \
  --output-path "../owlcms_schedule_scraper/vws2_final_start_list.ts" \
  --skip-verify
```

Expect: `Detected format: wso_table`, `Parsed 677 entries`.

## 2. Verify

Still from `final_start_scraper`:

```bash
venv/bin/python output_check.py \
  --pdf-url "https://assets.contentstack.io/v3/assets/blteb7d012fc7ebef7f/bltf77c5ae5086fcf79/6a972c04e1500a3542016f2a/2026_-_VWS2_-_Start_List.pdf" \
  --meet-name "2026 Virus Weightlifting Series 2, Powered by Rogue Fitness" \
  --output-path "../owlcms_schedule_scraper/vws2_final_start_list.ts" \
  --schedule-path "../owlcms_schedule_scraper/vws2_schedule_preview.ts"
```

Success criteria (see saved report):

- `Output rows: 677` and `Output matches parser: True`
- `weightClasses: 0`, `genders: 0`, `sessions: 0`, `memberIds: 0`, `names: 0`, `clubs: 0`
- `Missing sessions from schedule: 0`
- `Missing session/platform combos from schedule: 1` — only **session 18 Red** (empty in source PDF)

Do **not** block on differences vs older preview or Postgres snapshots; treat the PDF + verifier as source of truth.

## 3. Publish Athletes To Postgres

Requires `DATABASE_URL` (see repo `.env`).

**Athletes only** — delete all athletes for this meet, then upsert the scraped rows. Session schedule rows in Postgres are left unchanged.

Use the meet automation venv from `scrapers/`:

```bash
cd scrapers
set -a && source ../.env && set +a
PYTHONPATH=".:usaw/final_start_scraper" usaw/meet_automation/.venv/bin/python - <<'PY'
from pathlib import Path
import sys

sys.path.insert(0, "usaw/final_start_scraper")
from output_check import load_output_entries
from common import postgres_writer as pg

MEET = "2026 Virus Weightlifting Series 2, Powered by Rogue Fitness"
athletes = load_output_entries(
    Path("usaw/owlcms_schedule_scraper/vws2_final_start_list.ts")
)

with pg.connect() as conn:
    deleted = conn.execute("DELETE FROM athletes WHERE meet = %s", (MEET,)).rowcount
    print(f"Postgres deleted {deleted} athletes")
    for row in athletes:
        pg.upsert_athlete(conn, {**row, "meet": MEET})
    conn.commit()
    count = conn.execute(
        "SELECT COUNT(*) AS c FROM athletes WHERE meet = %s", (MEET,)
    ).fetchone()["c"]
    print(f"Postgres athlete count: {count}")
PY
```

Verify counts:

- Postgres: `SELECT COUNT(*) FROM athletes WHERE meet = '…'` → **677**
- Postgres schedule: `SELECT COUNT(*) FROM session_schedule WHERE meet = '…'` → **59** (unchanged)

## Files Touched In This Work

| File | Role |
| --- | --- |
| `final_start_scraper/wso_table_scraper.py` | New WSO-table parser |
| `final_start_scraper/scraper.py` | Auto-detect + extract `wso_table`; skip `assign_member_ids` when lot present |
| `final_start_scraper/output_check.py` | Verifier support for `wso_table` + schedule coverage |
| `final_start_scraper/format_notes.md` | WSO-table format notes |
| `owlcms_schedule_scraper/vws2_final_start_list.ts` | Canonical scraped output |
| `owlcms_schedule_scraper/vws2_final_start_list_report.txt` | Latest verification report |

## Debugging Order

If counts or fields look wrong:

1. Re-run `output_check.py` and read the report sections above.
2. For missing rows, dump unparsed lots from the PDF (entry cell pattern is usually the blocker).
3. For wrong `weightClass`, inspect `COMPETITIONS` — multi-category strings often bleed masters weights.
4. For wrong `entryTotal`, inspect the **Entry** column only; do not parse totals from `COMPETITIONS`.
5. For club noise, check `_clean_club` / OPEN bleed stripping.

See also `final_start_scraper/SKILL.md` for the general final start list maintenance workflow.
