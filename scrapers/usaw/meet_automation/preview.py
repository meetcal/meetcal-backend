"""Render a self-contained HTML preview of a staged bundle.

The page is intentionally a single file with inline CSS so it can be opened
straight from a phone (served from anywhere, or even airdropped). It shows the
validation report first, then the schedule, then the start list grouped by
session — exactly what you need to eyeball before approving.
"""

from __future__ import annotations

import html
from typing import Any, Dict, List, Optional

from .models import StagedBundle

_SEVERITY_COLOR = {"error": "#b00020", "warning": "#a86500", "info": "#5a5a5a"}


def _esc(value: Any) -> str:
    return html.escape("" if value is None else str(value))


def _session_key(row: Dict[str, Any]):
    try:
        sid = int(float(row.get("sessionId", row.get("session_id", 0)) or 0))
    except (TypeError, ValueError):
        sid = 0
    return (
        _esc(row.get("date", "")),
        sid,
        _esc(row.get("platform", "")),
    )


def _validation_section(validation: Optional[Dict[str, Any]]) -> str:
    if not validation:
        return "<p>No validation report.</p>"
    counts = validation.get("counts", {})
    status_color = "#0a7d28" if validation.get("ok") else "#b00020"
    status_text = "PASSED" if validation.get("ok") else "NEEDS REVIEW"
    rows = []
    for finding in validation.get("findings", []):
        color = _SEVERITY_COLOR.get(finding["severity"], "#333")
        examples = "<br>".join(_esc(e) for e in finding.get("examples", []))
        rows.append(
            f"<tr>"
            f"<td style='color:{color};font-weight:600'>{_esc(finding['severity'])}</td>"
            f"<td>{_esc(finding['message'])}</td>"
            f"<td style='text-align:right'>{_esc(finding['count'])}</td>"
            f"<td style='color:#555;font-size:12px'>{examples}</td>"
            f"</tr>"
        )
    findings_table = (
        "<table><thead><tr><th>Severity</th><th>Issue</th><th>#</th><th>Examples</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody></table>"
        if rows
        else "<p style='color:#0a7d28'>No issues found. 🎉</p>"
    )
    return f"""
      <div class="card">
        <h2>Validation <span style="color:{status_color}">{status_text}</span></h2>
        <p>
          <b>{counts.get('athletes', 0)}</b> athletes ·
          <b>{counts.get('schedule_rows', 0)}</b> schedule rows ·
          <b>{counts.get('sessions', 0)}</b> sessions ·
          platforms: {_esc(', '.join(counts.get('platforms', [])))}<br>
          errors: <b style="color:#b00020">{validation.get('errors', 0)}</b> ·
          warnings: <b style="color:#a86500">{validation.get('warnings', 0)}</b> ·
          no WSO: {counts.get('athletes_without_wso', 0)} ·
          no club: {counts.get('athletes_without_club', 0)}
        </p>
        {findings_table}
      </div>
    """


def _schedule_section(schedule: List[Dict[str, Any]]) -> str:
    rows = []
    for row in sorted(schedule, key=_session_key):
        rows.append(
            "<tr>"
            f"<td>{_esc(row.get('date'))}</td>"
            f"<td style='text-align:right'>{_esc(row.get('sessionId', row.get('session_id')))}</td>"
            f"<td>{_esc(row.get('platform'))}</td>"
            f"<td>{_esc(row.get('weighInTime', row.get('weigh_in_time')))}</td>"
            f"<td>{_esc(row.get('startTime', row.get('start_time')))}</td>"
            f"<td>{_esc(row.get('weightClass', row.get('weight_class')))}</td>"
            "</tr>"
        )
    return f"""
      <div class="card">
        <h2>Schedule ({len(schedule)})</h2>
        <table><thead><tr>
          <th>Date</th><th>Sess</th><th>Platform</th><th>Weigh-in</th><th>Start</th><th>Weight class</th>
        </tr></thead><tbody>{''.join(rows)}</tbody></table>
      </div>
    """


def _athletes_section(athletes: List[Dict[str, Any]]) -> str:
    def key(a: Dict[str, Any]):
        try:
            sid = int(float(a.get("sessionNumber", a.get("session_number", 0)) or 0))
        except (TypeError, ValueError):
            sid = 0
        return (sid, _esc(a.get("sessionPlatform", a.get("session_platform", ""))), _esc(a.get("name", "")))

    rows = []
    for a in sorted(athletes, key=key):
        rows.append(
            "<tr>"
            f"<td style='text-align:right'>{_esc(a.get('sessionNumber', a.get('session_number')))}</td>"
            f"<td>{_esc(a.get('sessionPlatform', a.get('session_platform')))}</td>"
            f"<td>{_esc(a.get('name'))}</td>"
            f"<td style='text-align:right'>{_esc(a.get('age'))}</td>"
            f"<td>{_esc(a.get('gender'))}</td>"
            f"<td>{_esc(a.get('weightClass', a.get('weight_class')))}</td>"
            f"<td style='text-align:right'>{_esc(a.get('entryTotal', a.get('entry_total')))}</td>"
            f"<td>{_esc(a.get('club'))}</td>"
            f"<td>{_esc(a.get('wso'))}</td>"
            "</tr>"
        )
    return f"""
      <div class="card">
        <h2>Start list ({len(athletes)})</h2>
        <table><thead><tr>
          <th>Sess</th><th>Plat</th><th>Name</th><th>Age</th><th>Gen</th>
          <th>WC</th><th>Total</th><th>Club</th><th>WSO</th>
        </tr></thead><tbody>{''.join(rows)}</tbody></table>
      </div>
    """


def render_preview(bundle: StagedBundle) -> str:
    src = bundle.source
    links = []
    if src.page_url:
        links.append(f'<a href="{_esc(src.page_url)}">meet page</a>')
    if src.start_list_url:
        links.append(f'<a href="{_esc(src.start_list_url)}">start-list PDF</a>')
    if src.schedule_url:
        links.append(f'<a href="{_esc(src.schedule_url)}">schedule PDF</a>')
    links_html = " · ".join(links)

    return f"""<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{_esc(bundle.meet_name)} — staged preview</title>
<style>
  body {{ font-family: -apple-system, system-ui, sans-serif; margin: 0; background: #f5f5f7; color: #111; }}
  header {{ background: #111; color: #fff; padding: 16px; position: sticky; top: 0; }}
  header h1 {{ font-size: 18px; margin: 0 0 4px; }}
  header .meta {{ font-size: 12px; color: #bbb; }}
  header a {{ color: #6db3ff; }}
  .wrap {{ padding: 12px; max-width: 1000px; margin: 0 auto; }}
  .card {{ background: #fff; border-radius: 10px; padding: 14px; margin-bottom: 14px; box-shadow: 0 1px 3px rgba(0,0,0,.08); }}
  .card h2 {{ font-size: 15px; margin: 0 0 10px; }}
  table {{ border-collapse: collapse; width: 100%; font-size: 13px; }}
  th, td {{ text-align: left; padding: 6px 8px; border-bottom: 1px solid #eee; vertical-align: top; }}
  th {{ position: sticky; top: 0; background: #fafafa; font-size: 11px; text-transform: uppercase; color: #666; }}
  tbody tr:nth-child(odd) {{ background: #fcfcfd; }}
</style></head>
<body>
  <header>
    <h1>{_esc(bundle.meet_name)}</h1>
    <div class="meta">run {_esc(bundle.run_id)} · staged {_esc(bundle.created_at)} · status <b>{_esc(bundle.status)}</b><br>{links_html}</div>
  </header>
  <div class="wrap">
    {_validation_section(bundle.validation)}
    {_schedule_section(bundle.schedule)}
    {_athletes_section(bundle.athletes)}
  </div>
</body></html>
"""
