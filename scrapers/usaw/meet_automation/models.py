"""Canonical staged data shapes shared across the pipeline.

Athletes and schedule rows are kept in camelCase ingest shape that
``postgres_writer`` already accepts, so no translation is needed downstream.
"""

from __future__ import annotations

import json
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional

# Lifecycle of a staged run.
STATUS_PENDING_APPROVAL = "pending_approval"
STATUS_APPROVED = "approved"
STATUS_REJECTED = "rejected"
STATUS_INGESTED = "ingested"
STATUS_FAILED = "failed"


def now_iso() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


@dataclass
class SourceRefs:
    page_url: str
    start_list_url: Optional[str] = None
    schedule_url: Optional[str] = None
    start_list_sha256: Optional[str] = None
    schedule_sha256: Optional[str] = None


@dataclass
class SlackRef:
    channel: Optional[str] = None
    ts: Optional[str] = None  # thread root timestamp of the review message


@dataclass
class StagedBundle:
    """Everything one detect->scrape->validate run produced, on disk."""

    run_id: str
    watch_key: str
    meet_name: str
    created_at: str = field(default_factory=now_iso)
    status: str = STATUS_PENDING_APPROVAL
    source: SourceRefs = field(default_factory=lambda: SourceRefs(page_url=""))
    athletes: List[Dict[str, Any]] = field(default_factory=list)
    schedule: List[Dict[str, Any]] = field(default_factory=list)
    meet: Optional[Dict[str, Any]] = None
    scrape_stats: Dict[str, Any] = field(default_factory=dict)
    validation: Optional[Dict[str, Any]] = None
    slack: SlackRef = field(default_factory=SlackRef)
    ingest_result: Optional[Dict[str, Any]] = None

    # --- persistence -----------------------------------------------------
    def to_dict(self) -> Dict[str, Any]:
        data = asdict(self)
        return data

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "StagedBundle":
        source = SourceRefs(**data.get("source", {"page_url": ""}))
        slack = SlackRef(**data.get("slack", {}))
        return cls(
            run_id=data["run_id"],
            watch_key=data["watch_key"],
            meet_name=data["meet_name"],
            created_at=data.get("created_at", now_iso()),
            status=data.get("status", STATUS_PENDING_APPROVAL),
            source=source,
            athletes=data.get("athletes", []),
            schedule=data.get("schedule", []),
            meet=data.get("meet"),
            scrape_stats=data.get("scrape_stats", {}),
            validation=data.get("validation"),
            slack=slack,
            ingest_result=data.get("ingest_result"),
        )

    def save(self, run_dir: Path) -> Path:
        run_dir.mkdir(parents=True, exist_ok=True)
        path = run_dir / "bundle.json"
        path.write_text(json.dumps(self.to_dict(), indent=2), encoding="utf-8")
        return path

    @classmethod
    def load(cls, run_dir: Path) -> "StagedBundle":
        path = run_dir / "bundle.json"
        return cls.from_dict(json.loads(path.read_text(encoding="utf-8")))
