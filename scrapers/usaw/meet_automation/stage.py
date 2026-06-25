"""Persist a staged run to disk: bundle.json + preview.html.

Staging never touches a database. A run lives under
``state/runs/<run_id>/`` and is the unit a human approves.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import List

from . import config
from .models import StagedBundle
from .preview import render_preview


def make_run_id(watch_key: str) -> str:
    return f"{watch_key}-{time.strftime('%Y%m%d-%H%M%S', time.gmtime())}"


def run_dir_for(run_id: str) -> Path:
    return config.RUNS_DIR / run_id


def write_run(bundle: StagedBundle) -> Path:
    run_dir = run_dir_for(bundle.run_id)
    bundle.save(run_dir)
    (run_dir / "preview.html").write_text(render_preview(bundle), encoding="utf-8")
    return run_dir


def list_runs() -> List[str]:
    if not config.RUNS_DIR.exists():
        return []
    return sorted(p.name for p in config.RUNS_DIR.iterdir() if p.is_dir())


def load_run(run_id: str) -> StagedBundle:
    return StagedBundle.load(run_dir_for(run_id))
