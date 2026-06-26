"""Command-line entry point for the meet automation pipeline.

Typical wiring (cron):

    # check for new/changed PDFs, stage + notify (no DB writes):
    python -m usaw.meet_automation.pipeline run --all

    # poll Slack for your "okay"/"reject" reply and publish on approval:
    python -m usaw.meet_automation.pipeline approve --all-pending

Manual operation:

    python -m usaw.meet_automation.pipeline list
    python -m usaw.meet_automation.pipeline show <run_id>
    python -m usaw.meet_automation.pipeline ingest <run_id>     # publish now
    python -m usaw.meet_automation.pipeline reject <run_id>     # discard
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import List, Optional

from . import config, detect, ingest, scrape, slack, stage
from .config import MeetWatch, SlackConfig
from .models import (
    STATUS_APPROVED,
    STATUS_INGESTED,
    STATUS_PENDING_APPROVAL,
    STATUS_REJECTED,
    SourceRefs,
    StagedBundle,
)
from .validate import validate


def _watches(args) -> List[MeetWatch]:
    if getattr(args, "watch", None):
        w = config.find_watch(args.watch)
        if not w:
            raise SystemExit(f"no watch with key '{args.watch}' in watches.json")
        return [w]
    if getattr(args, "all", False):
        watches = config.load_watches()
        if not watches:
            raise SystemExit("no watches defined in watches.json")
        return watches
    raise SystemExit("specify --watch <key> or --all")


# --------------------------------------------------------------------------
def cmd_detect(args) -> int:
    for watch in _watches(args):
        result = detect.detect(watch, hash_pdfs=not args.no_hash)
        print(f"[{watch.key}] changed={result.changed} reasons={result.reasons}")
        print(f"  start_list: {result.start_list_url}")
        print(f"  schedule:   {result.schedule_url}")
        if args.candidates:
            print(f"  candidates: {result.candidates}")
    return 0


def cmd_run(args) -> int:
    slack_cfg = SlackConfig.from_env()
    if getattr(args, "requested", False):
        return _run_requested(args, slack_cfg)
    failures = 0
    for watch in _watches(args):
        try:
            _run_one(watch, args, slack_cfg)
        except Exception as exc:  # noqa: BLE001
            failures += 1
            print(f"[{watch.key}] FAILED: {exc}", file=sys.stderr)
    return 1 if failures else 0


def _run_requests_dir() -> Path:
    """Resolved at call time so tests can redirect ``config.STATE_DIR``."""
    return config.STATE_DIR / config.RUN_REQUESTS_DIRNAME


def _resolve_run_request(data: dict, stem: str, watches_by_key: dict):
    """Turn one run-request file into (watches, force).

    The Rust ``/meet-run`` endpoint writes ``{key, all, force}``; ``all`` runs
    every watch, otherwise the single ``key`` (falling back to the filename
    stem). Returns ``([], force)`` if a named watch no longer exists so the
    caller can drop the stale request.
    """
    force = bool(data.get("force", True))
    if data.get("all"):
        return list(watches_by_key.values()), force
    key = data.get("key") or stem
    watch = watches_by_key.get(key)
    return ([watch] if watch else []), force


def _run_requested(args, slack_cfg: SlackConfig) -> int:
    """Drain `/meet-run` requests dropped by the Slack command. Each request
    file is consumed (deleted) whether or not its run succeeds, so a stuck watch
    can't wedge the queue; the requester just re-runs `/meet-run`."""
    req_dir = _run_requests_dir()
    paths = sorted(req_dir.glob("*.json")) if req_dir.exists() else []
    if not paths:
        print("no run requests")
        return 0

    watches_by_key = {w.key: w for w in config.load_watches()}
    failures = 0
    for path in paths:
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except Exception as exc:  # noqa: BLE001
            print(f"[{path.name}] bad request file: {exc}", file=sys.stderr)
            _unlink(path)
            continue

        watches, force = _resolve_run_request(data, path.stem, watches_by_key)
        if not watches:
            print(f"[{path.stem}] no matching watch; dropping request", file=sys.stderr)
            _unlink(path)
            continue

        # Per-request overrides; a manual trigger forces a fresh stage.
        req_args = argparse.Namespace(
            force=force,
            allow_empty=bool(data.get("allow_empty", False)),
            no_slack=getattr(args, "no_slack", False),
        )
        for watch in watches:
            try:
                _run_one(watch, req_args, slack_cfg)
            except Exception as exc:  # noqa: BLE001
                failures += 1
                print(f"[{watch.key}] FAILED: {exc}", file=sys.stderr)
        _unlink(path)
    return 1 if failures else 0


def _unlink(path: Path) -> None:
    try:
        path.unlink()
    except FileNotFoundError:
        pass


def _run_one(watch: MeetWatch, args, slack_cfg: SlackConfig) -> None:
    result = detect.detect(watch, hash_pdfs=True)
    if not result.changed and not args.force:
        print(f"[{watch.key}] no change; skipping (use --force to stage anyway)")
        return
    if not (result.start_list_url and result.schedule_url):
        print(f"[{watch.key}] missing PDF(s): {result.reasons}", file=sys.stderr)
        return

    print(f"[{watch.key}] change detected: {result.reasons}")
    start_bytes = detect.fetch_bytes(result.start_list_url)
    sched_bytes = detect.fetch_bytes(result.schedule_url)

    athletes, schedule, scrape_stats = scrape.scrape(
        watch, start_bytes, sched_bytes, start_list_url=result.start_list_url
    )
    report = validate(athletes, schedule, watch.meet_name)

    if not athletes and not getattr(args, "allow_empty", False):
        print(
            f"[{watch.key}] parsed 0 athletes; not staging "
            f"(use --force --allow-empty to stage anyway)",
            file=sys.stderr,
        )
        return

    run_id = stage.make_run_id(watch.key)
    bundle = StagedBundle(
        run_id=run_id,
        watch_key=watch.key,
        meet_name=watch.meet_name,
        status=STATUS_PENDING_APPROVAL,
        source=SourceRefs(
            page_url=watch.page_url,
            start_list_url=result.start_list_url,
            schedule_url=result.schedule_url,
            start_list_sha256=result.start_list_sha256,
            schedule_sha256=result.schedule_sha256,
        ),
        athletes=athletes,
        schedule=schedule,
        meet=None,  # meet/venue details are owned by the meet-sync job
        scrape_stats=scrape_stats,
        validation=report,
    )
    run_dir = stage.write_run(bundle)
    print(
        f"[{watch.key}] staged {run_id}: {report['counts']['athletes']} athletes, "
        f"{report['counts']['schedule_rows']} schedule rows, "
        f"errors={report['errors']} warnings={report['warnings']}"
    )
    print(f"  preview: {run_dir / 'preview.html'}")

    if not args.no_slack:
        try:
            ref = slack.post_review(slack_cfg, bundle)
            bundle.slack = ref
            bundle.save(run_dir)
            print(f"  slack: posted (channel={ref.channel} ts={ref.ts})")
        except Exception as exc:  # noqa: BLE001
            print(f"  slack: NOT posted ({exc})", file=sys.stderr)

    # Only mark seen after a successful stage, so a transient failure re-tries.
    detect.mark_seen(result)


def _format_target_confirmation(run_id: str, target: str, stats: dict) -> str:
    if target == "postgres":
        a = stats.get("athletes", {})
        s = stats.get("schedule", {})
        msg = (
            f":white_check_mark: *Postgres* updated for `{run_id}` — "
            f"athletes: {a.get('inserted', 0)} ins / {a.get('updated', 0)} upd, "
            f"schedule: {s.get('inserted', 0)} ins / {s.get('updated', 0)} upd"
        )
        if stats.get("deleted_athletes") or stats.get("deleted_schedule"):
            msg += (
                f" (replaced {stats.get('deleted_athletes', 0)} athletes, "
                f"{stats.get('deleted_schedule', 0)} schedule rows)"
            )
        return msg
    if target == "convex":
        msg = (
            f":white_check_mark: *Convex* updated for `{run_id}` — "
            f"athletes: {stats.get('athletes', 0)}, schedule: {stats.get('schedule', 0)}"
        )
        if stats.get("failed"):
            msg += f" :warning: {stats['failed']} failed"
        return msg
    return f":white_check_mark: *{target}* updated for `{run_id}`"


def _slack_target_reporter(slack_cfg: SlackConfig, bundle: StagedBundle):
    """Post one Slack confirmation per DB as each upload completes."""
    def report(target: str, stats: dict) -> None:
        try:
            slack.post_thread_reply(
                slack_cfg, bundle, _format_target_confirmation(bundle.run_id, target, stats)
            )
        except Exception:  # noqa: BLE001
            pass
    return report


def _do_ingest(
    bundle: StagedBundle, target: str, replace: bool, slack_cfg: Optional[SlackConfig] = None
) -> StagedBundle:
    on_target_done = (
        _slack_target_reporter(slack_cfg, bundle)
        if slack_cfg is not None and bundle.slack.ts
        else None
    )
    result = ingest.ingest_bundle(
        bundle.athletes, bundle.schedule, bundle.meet, bundle.meet_name,
        target=target, replace=replace, on_target_done=on_target_done,
    )
    bundle.ingest_result = result
    bundle.status = STATUS_INGESTED
    stage.write_run(bundle)
    return bundle


def cmd_ingest(args) -> int:
    bundle = stage.load_run(args.run_id)
    if bundle.status == STATUS_INGESTED and not args.force:
        print(f"{args.run_id} already ingested; use --force to re-ingest")
        return 0
    bundle = _do_ingest(bundle, args.target, replace=not args.no_replace, slack_cfg=SlackConfig.from_env())
    print(f"ingested {args.run_id}: {bundle.ingest_result}")
    return 0


def cmd_reject(args) -> int:
    bundle = stage.load_run(args.run_id)
    bundle.status = STATUS_REJECTED
    stage.write_run(bundle)
    print(f"rejected {args.run_id}")
    return 0


def _read_decision(run_id: str):
    """Return (decision, path) from a button-click decision file, if any.

    The Rust interactions endpoint writes <state>/decisions/<run_id>.json when a
    button is clicked. This is the primary approval signal; Slack reply polling
    is the fallback.
    """
    path = config.STATE_DIR / "decisions" / f"{run_id}.json"
    if not path.exists():
        return None, None
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return data.get("decision"), path
    except Exception:  # noqa: BLE001
        return None, path


def _consume_decision(decision_path) -> None:
    if decision_path is not None:
        try:
            decision_path.unlink()
        except FileNotFoundError:
            pass


def cmd_approve(args) -> int:
    slack_cfg = SlackConfig.from_env()
    run_ids = [args.run_id] if args.run_id else stage.list_runs()
    acted = False
    for run_id in run_ids:
        bundle = stage.load_run(run_id)
        if bundle.status != STATUS_PENDING_APPROVAL:
            continue
        decision, decision_path = _read_decision(run_id)
        if decision is None:
            decision = slack.poll_approval(slack_cfg, bundle)
        if decision == "approved":
            print(f"[{run_id}] approved in Slack -> ingesting")
            bundle.status = STATUS_APPROVED
            try:
                slack.post_thread_reply(slack_cfg, bundle, f":rocket: Approved — publishing `{run_id}`…")
            except Exception:  # noqa: BLE001
                pass
            # _do_ingest posts one confirmation per DB as each upload completes.
            bundle = _do_ingest(bundle, args.target, replace=not args.no_replace, slack_cfg=slack_cfg)
            try:
                slack.post_thread_reply(slack_cfg, bundle, f":checkered_flag: `{run_id}` published to all targets.")
            except Exception:  # noqa: BLE001
                pass
            _consume_decision(decision_path)
            acted = True
        elif decision == "rejected":
            print(f"[{run_id}] rejected in Slack")
            bundle.status = STATUS_REJECTED
            stage.write_run(bundle)
            try:
                slack.post_thread_reply(slack_cfg, bundle, f":wastebasket: Discarded `{run_id}`.")
            except Exception:  # noqa: BLE001
                pass
            _consume_decision(decision_path)
            acted = True
        else:
            print(f"[{run_id}] still pending")
    if not acted:
        print("no runs acted on")
    return 0


def cmd_list(args) -> int:
    for run_id in stage.list_runs():
        try:
            bundle = stage.load_run(run_id)
        except Exception:  # noqa: BLE001
            continue
        v = bundle.validation or {}
        print(
            f"{run_id}\t{bundle.status}\tathletes={v.get('counts', {}).get('athletes', 0)}\t"
            f"errors={v.get('errors', 0)}\twarnings={v.get('warnings', 0)}"
        )
    return 0


def cmd_show(args) -> int:
    bundle = stage.load_run(args.run_id)
    print(f"run_id:     {bundle.run_id}")
    print(f"meet:       {bundle.meet_name}")
    print(f"status:     {bundle.status}")
    print(f"created_at: {bundle.created_at}")
    print(f"source:     {bundle.source}")
    print(f"counts:     {(bundle.validation or {}).get('counts')}")
    print(f"preview:    {stage.run_dir_for(bundle.run_id) / 'preview.html'}")
    print(f"slack:      channel={bundle.slack.channel} ts={bundle.slack.ts}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="meet_automation.pipeline")
    sub = p.add_subparsers(dest="command", required=True)

    def add_target(sp):
        sp.add_argument("--target", choices=["both", "postgres", "convex"], default="both")
        sp.add_argument("--no-replace", action="store_true",
                        help="upsert without deleting existing rows for the meet first")

    d = sub.add_parser("detect", help="check for new/changed PDFs")
    d.add_argument("--watch")
    d.add_argument("--all", action="store_true")
    d.add_argument("--no-hash", action="store_true", help="skip downloading PDFs to hash")
    d.add_argument("--candidates", action="store_true", help="print all discovered PDF links")
    d.set_defaults(func=cmd_detect)

    r = sub.add_parser("run", help="detect -> scrape -> validate -> stage -> notify")
    r.add_argument("--watch")
    r.add_argument("--all", action="store_true")
    r.add_argument(
        "--requested",
        action="store_true",
        help="drain `/meet-run` requests dropped by the Slack command",
    )
    r.add_argument("--force", action="store_true", help="stage even if unchanged")
    r.add_argument("--no-slack", action="store_true")
    r.add_argument("--allow-empty", action="store_true")
    r.set_defaults(func=cmd_run)

    a = sub.add_parser("approve", help="poll Slack replies/buttons and publish approved runs")
    a.add_argument("run_id", nargs="?")
    a.add_argument(
        "--all-pending",
        action="store_true",
        help="process every pending run (the default when no run_id is given)",
    )
    add_target(a)
    a.set_defaults(func=cmd_approve)

    i = sub.add_parser("ingest", help="publish a staged run now (manual approval)")
    i.add_argument("run_id")
    i.add_argument("--force", action="store_true")
    add_target(i)
    i.set_defaults(func=cmd_ingest)

    j = sub.add_parser("reject", help="discard a staged run")
    j.add_argument("run_id")
    j.set_defaults(func=cmd_reject)

    sub.add_parser("list", help="list staged runs").set_defaults(func=cmd_list)

    s = sub.add_parser("show", help="show one staged run")
    s.add_argument("run_id")
    s.set_defaults(func=cmd_show)

    return p


def main(argv: Optional[List[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
