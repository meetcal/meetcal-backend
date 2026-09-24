#!/usr/bin/env python3
"""One-off, idempotent cleanup of duplicate id-less athlete rows.

Before the entry scraper minted deterministic ``noid:`` placeholders, an
athlete without a membership number got a fresh random nine-digit
``member_id`` on every nightly run, so the same person accumulated one
``athletes`` row per run. This script collapses those groups.

A group is collapsed only when it is unambiguous:

* same meet, same normalised name, same gender, same age (the nightly
  copies came from one entry row; two different people who share a name
  almost never share an age as well);
* more than one row;
* every ``member_id`` in the group is blank, a ``noid:`` placeholder, or a
  nine-digit number in the range ``Math.random()`` used to mint them; and
* none of those numbers appears in any other meet (a real membership number
  recurs across meets; a random one never does).

Within a group the row that already carries a session assignment is kept
(it is the one the schedule pipeline wrote); otherwise the newest row. A
blank kept ``member_id`` becomes the ``noid:`` placeholder; a nine-digit one
is left as it is (the writer's id-less lookup matches it, and it may be a
real first-meet membership number). Re-running finds nothing to do, which is
what makes it safe to run twice.

Dry run by default. Usage:

    DATABASE_URL=... python -m common.dedupe_idless_athletes --meet "<meet>"
    DATABASE_URL=... python -m common.dedupe_idless_athletes --all-meets --apply
"""

from __future__ import annotations

import argparse
import re
import sys
from collections import defaultdict
from typing import Any

from common import postgres_writer as pg
from common.normalize import (
    is_placeholder_member_id,
    normalize_name,
    placeholder_member_id,
)

# `String(Math.floor(Math.random() * 900000000) + 100000000)`: nine digits,
# never leading with zero.
RANDOM_MEMBER_ID_RE = re.compile(r"^[1-9]\d{8}$")


def _is_collapsible_member_id(member_id: Any) -> bool:
    if is_placeholder_member_id(member_id):
        return True
    return isinstance(member_id, str) and bool(RANDOM_MEMBER_ID_RE.match(member_id.strip()))


def _pick_keeper(rows: list[dict[str, Any]]) -> dict[str, Any]:
    assigned = [row for row in rows if pg.athlete_has_session_assignment(row)]
    pool = assigned or rows
    return max(pool, key=lambda row: row["id"])


def plan_meet(conn, meet: str) -> list[dict[str, Any]]:
    """Return the groups that would be collapsed for ``meet``; writes nothing."""
    meet = pg.require_text(meet, "meet")
    rows = conn.execute(
        """
        SELECT id, member_id, name, gender, age, session_number, session_platform
        FROM athletes
        WHERE meet = %s
        ORDER BY id
        """,
        (meet,),
    ).fetchall()

    groups: dict[tuple[str, str, Any], list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        groups[(normalize_name(row["name"]), row["gender"] or "", row["age"])].append(row)

    plans = []
    for (normalized, gender, _age), members in groups.items():
        if len(members) < 2:
            continue
        if not all(_is_collapsible_member_id(row["member_id"]) for row in members):
            continue
        numeric_ids = [
            row["member_id"].strip()
            for row in members
            if not is_placeholder_member_id(row["member_id"])
        ]
        if numeric_ids:
            elsewhere = conn.execute(
                "SELECT 1 FROM athletes WHERE member_id = ANY(%s) AND meet <> %s LIMIT 1",
                (numeric_ids, meet),
            ).fetchone()
            if elsewhere:
                continue
        keeper = _pick_keeper(members)
        plans.append(
            {
                "meet": meet,
                "name": keeper["name"],
                "normalized_name": normalized,
                "gender": gender,
                "keep_id": keeper["id"],
                "delete_ids": [row["id"] for row in members if row["id"] != keeper["id"]],
                "member_id": (keeper["member_id"] or "").strip()
                or placeholder_member_id(keeper["name"]),
            }
        )
    return plans


def dedupe_meet(conn, meet: str, *, apply: bool) -> dict[str, Any]:
    """Collapse duplicate id-less athletes for one meet.

    With ``apply=False`` nothing is written. With ``apply=True`` the deletes
    and member_id rewrites run on ``conn``; the caller commits.
    """
    plans = plan_meet(conn, meet)
    deleted = 0
    if apply:
        for plan in plans:
            deleted += conn.execute(
                "DELETE FROM athletes WHERE meet = %s AND id = ANY(%s)",
                (meet, plan["delete_ids"]),
            ).rowcount
            conn.execute(
                "UPDATE athletes SET member_id = %s WHERE id = %s",
                (plan["member_id"], plan["keep_id"]),
            )
    return {"meet": meet, "groups": plans, "deleted": deleted, "applied": apply}


def _all_meets(conn) -> list[str]:
    return [
        row["meet"]
        for row in conn.execute("SELECT DISTINCT meet FROM athletes ORDER BY meet").fetchall()
    ]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--meet", help="collapse duplicates for this meet only")
    target.add_argument("--all-meets", action="store_true", help="every meet in athletes")
    parser.add_argument("--apply", action="store_true", help="write changes (default: dry run)")
    args = parser.parse_args(argv)

    with pg.connect() as conn:
        meets = _all_meets(conn) if args.all_meets else [args.meet]
        total_groups = 0
        total_deleted = 0
        for meet in meets:
            summary = dedupe_meet(conn, meet, apply=args.apply)
            for plan in summary["groups"]:
                total_groups += 1
                verb = "collapsed" if args.apply else "would collapse"
                print(
                    f"[{meet}] {verb} '{plan['name']}' ({plan['gender']}): keep id={plan['keep_id']} "
                    f"as {plan['member_id']}, delete ids={plan['delete_ids']}"
                )
            total_deleted += summary["deleted"]
        if args.apply:
            conn.commit()
            print(f"Collapsed {total_groups} group(s); deleted {total_deleted} row(s).")
        else:
            print(f"Dry run: {total_groups} group(s) would be collapsed. Re-run with --apply.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
