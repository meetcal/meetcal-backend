#!/usr/bin/env bash
set -euo pipefail

ROOT="${MEETCAL_BACKEND_ROOT:-/home/maddisen/dev/meetcal-llc/meetcal-backend}"
ENV_FILE="${ENV_FILE:-${ROOT}/.env}"
SCRAPERS_DIR="${ROOT}/scrapers"
JOB="${1:-}"

if [[ -z "${JOB}" ]]; then
  echo >&2 "Usage: $0 <job>"
  exit 2
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

export CONVEX_URL="${CONVEX_URL:-postgres-cron}"
export SCRAPER_SECRET="${SCRAPER_SECRET:-${SCAPER_SECRET:-postgres-cron}}"
export SLACK_IWF_RECORDS_WEBHOOK_URL="${SLACK_IWF_RECORDS_WEBHOOK_URL:-${SLACK_RECORDS_WEBHOOK_URL:-}}"
export PYTHONPATH="${SCRAPERS_DIR}${PYTHONPATH:+:${PYTHONPATH}}"

LOCK_DIR="${SCRAPERS_DIR}/.locks"
mkdir -p "${LOCK_DIR}"
exec 9>"${LOCK_DIR}/${JOB}.lock"
if ! flock -n 9; then
  echo "Job ${JOB} is already running; skipping."
  exit 0
fi

python_job() {
  local dir="$1"
  shift
  local venv="${dir}/.venv"
  local stamp="${venv}/.requirements.stamp"
  local root_req="${SCRAPERS_DIR}/requirements.txt"
  local local_req="${dir}/requirements.txt"
  local install=0

  if [[ ! -x "${venv}/bin/python" ]]; then
    python3 -m venv "${venv}"
    install=1
  fi

  if [[ ! -f "${stamp}" ]] || [[ "${root_req}" -nt "${stamp}" ]] || { [[ -f "${local_req}" ]] && [[ "${local_req}" -nt "${stamp}" ]]; }; then
    install=1
  fi

  if [[ "${install}" == "1" ]]; then
    "${venv}/bin/python" -m pip install --upgrade pip
    "${venv}/bin/python" -m pip install -r "${root_req}"
    if [[ -f "${local_req}" ]]; then
      "${venv}/bin/python" -m pip install -r "${local_req}"
    fi
    date -u +"%Y-%m-%dT%H:%M:%SZ" > "${stamp}"
  fi

  (cd "${dir}" && "${venv}/bin/python" "$@")
}

node_job() {
  local dir="$1"
  shift
  if [[ -f "${dir}/package-lock.json" ]]; then
    (cd "${dir}" && npm ci)
  elif [[ -f "${dir}/package.json" && ! -d "${dir}/node_modules" ]]; then
    (cd "${dir}" && npm install)
  fi
  (cd "${dir}" && node "$@")
}

ensure_postgres_ingest_python() {
  local dir="${SCRAPERS_DIR}/common"
  local venv="${dir}/.venv"
  local stamp="${venv}/.requirements.stamp"
  local root_req="${SCRAPERS_DIR}/requirements.txt"
  local install=0

  if [[ ! -x "${venv}/bin/python" ]]; then
    python3 -m venv "${venv}"
    install=1
  fi

  if [[ ! -f "${stamp}" ]] || [[ "${root_req}" -nt "${stamp}" ]]; then
    install=1
  fi

  if [[ "${install}" == "1" ]]; then
    "${venv}/bin/python" -m pip install --upgrade pip
    "${venv}/bin/python" -m pip install -r "${root_req}"
    date -u +"%Y-%m-%dT%H:%M:%SZ" > "${stamp}"
  fi

  export POSTGRES_INGEST_PYTHON="${venv}/bin/python"
}

entry_scrapers() {
  ensure_postgres_ingest_python
  local dir="${SCRAPERS_DIR}/usaw/entry_scraper"
  if [[ ! -d "${dir}/node_modules" ]]; then
    (cd "${dir}" && npm ci && npx playwright install chromium)
  fi

  # Managed entry targets: edited live via the Slack /entries-* commands on the
  # API server (writes entries_targets.json). The cron just reads whatever is in
  # that file each run -- no redeploy/git pull needed. Falls back to the built-in
  # list when the file is absent or empty so nothing breaks before adoption.
  local targets_file="${ENTRIES_TARGETS_PATH:-${dir}/entries_targets.json}"
  local urls=()
  if [[ -f "${targets_file}" ]]; then
    mapfile -t urls < <(python3 -c "import json,sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    sys.exit(0)
for item in (data if isinstance(data,list) else []):
    url=item.get('url') if isinstance(item,dict) else None
    if url: print(url)
" "${targets_file}" 2>/dev/null) || true
  fi

  if [[ ${#urls[@]} -eq 0 ]]; then
    echo "entries: no managed targets in ${targets_file}; using built-in fallback list"
    urls=(
      "https://usaweightlifting.sport80.com/public/events/14711/entries/21593?bl="
      "https://usaweightlifting.sport80.com/public/events/14508/entries/21398?bl="
      "https://usaweightlifting.sport80.com/public/events/14372/entries/21259?bl=locator"
      "https://usaweightlifting.sport80.com/public/events/14712/entries/21595?bl="
      "https://usaweightlifting.sport80.com/public/events/14522/entries/21416?bl="
      "https://usaweightlifting.sport80.com/public/events/14725/entries/21608?bl="
      "https://usaweightlifting.sport80.com/public/events/14723/entries/21606?bl="
      "https://usaweightlifting.sport80.com/public/events/14772/entries/21653?bl="
      "https://usaweightlifting.sport80.com/public/events/14510/entries/21402?bl="
      "https://usaweightlifting.sport80.com/public/events/14849/entries/21724?bl=locator"
    )
  fi

  export SLACK_WEBHOOK_URL="${SLACK_ENTRY_WEBHOOK_URL:-${SLACK_WEBHOOK_URL:-}}"
  for url in "${urls[@]}"; do
    printf '%s\n' "${url}" > "${dir}/target_url.txt"
    (cd "${dir}" && node csv_scraper.js)
  done
}

meet_sync() {
  ensure_postgres_ingest_python
  local dir="${SCRAPERS_DIR}/usaw/meet_to_supabase"
  export SLACK_WEBHOOK_URL="${SLACK_MEET_WEBHOOK_URL:-${SLACK_WEBHOOK_URL:-}}"
  node_job "${dir}" scripts/sync-meets.js
  node_job "${dir}" scripts/sync-nat-meets.js
  node_job "${dir}" scripts/sync-virus-meets.js
}

wso_scrapers() {
  local dir="${SCRAPERS_DIR}/usaw/wso_sheets_scraper"
  export SLACK_WEBHOOK_URL="${SLACK_WSO_WEBHOOK_URL:-${SLACK_WEBHOOK_URL:-}}"
  python_job "${dir}" auto_scrapers/scraper_dmv.py --wso "DMV" --sheet-url "https://docs.google.com/spreadsheets/d/1vYD2H6si9FyEO-Tc24DoFZOmST0r5hCn/edit?gid=799684986#gid=799684986"
  python_job "${dir}" auto_scrapers/scraper_florida.py --wso "Florida" --sheet-url "https://docs.google.com/spreadsheets/d/16sNrOTnGrGeXE4L5skgCfE5vLTA7ggpaHWfMQNh0DfQ/view?gid=490899077#gid=490899077"
  python_job "${dir}" auto_scrapers/scraper_tnky.py --wso "Tennessee-Kentucky" --sheet-url "https://docs.google.com/spreadsheets/d/11uUA0t05sEvHRjvDksC0VP1Yr2p_rC0JjHgVPEuYzhU/view?gid=867133960#gid=867133960"
  python_job "${dir}" auto_scrapers/scraper_carolinas.py --wso "Carolina" --sheet-url "https://docs.google.com/spreadsheets/d/1rKFzpkLCT-FE2SzM0qpUOoZ788YHl7dg/view?gid=1785893123#gid=1785893123"
  python_job "${dir}" auto_scrapers/scraper_ohio.py --wso "Ohio" --sheet-url "https://docs.google.com/spreadsheets/d/1fX-Ft3PuLn8BCE2thhwPEXFTEUTN7yJGxWi7LMajAD8/view?gid=0#gid=0"
  python_job "${dir}" auto_scrapers/scraper_newjersey.py --wso "New Jersey" --sheet-url "https://docs.google.com/spreadsheets/d/1y8mXDBLfqmszlzWhv-4wkeWQZS5Kb9Aj4RnB39CBJmw/edit?gid=0#gid=0"
  python_job "${dir}" auto_scrapers/scraper_ga_pnw.py --wso "Georgia" --sheet-url "https://docs.google.com/spreadsheets/d/1HM1H51pUmhoWDdSUp2RT-mCaUX2a8NB7aUSYVwWT0AU/edit?gid=908416148#gid=908416148"
  python_job "${dir}" auto_scrapers/scraper_pawv.py --wso "Pennsylvania-West Virginia" --sheet-id "2PACX-1vR8exp9-mwi8dpkZa9-48G-CUVuZ5rAlpOYdMCiNMka25wZ6V2XPLurpgMDtyiarqnQxYrW6dWfQ042"
  python_job "${dir}" auto_scrapers/scraper_ga_pnw.py --wso "Pacific Northwest" --sheet-url "https://docs.google.com/spreadsheets/d/1pmZ1j3KJyms0Dlk3xz_VVf6mWq6tqdZj/edit?gid=1648178012#gid=1648178012"
  python_job "${dir}" auto_scrapers/scraper_ga_pnw.py --wso "California North" --sheet-url "https://docs.google.com/spreadsheets/d/1ZAs27jQCPYTVgLuQ-feBHSO-BgGjGCewUs0djG23pXQ/edit?gid=35344992#gid=35344992"
  python_job "${dir}" auto_scrapers/scraper_newengland_auto.py
  python_job "${dir}" auto_scrapers/scraper_mountainsouth_auto.py
  python_job "${dir}" auto_scrapers/scraper_newyork_auto.py
  python_job "${dir}" auto_scrapers/scraper_illinois_auto.py
}

results_sport80() {
  export SLACK_WEBHOOK_URL="${SLACK_RESULTS_WEBHOOK_URL:-${SLACK_WEBHOOK_URL:-}}"
  python_job "${SCRAPERS_DIR}/usaw/sport80_api" update_supabase_from_sport80.py
}

usamw_events() {
  export SLACK_WEBHOOK_URL="${SLACK_MEET_WEBHOOK_URL:-${SLACK_WEBHOOK_URL:-}}"
  python_job "${SCRAPERS_DIR}/usamw/meets" scrape_events.py
}

meet_automation() {
  local dir="${SCRAPERS_DIR}/usaw/meet_automation"
  shift || true
  python_job "${dir}" -m usaw.meet_automation.pipeline "$@"
}

meet_automation_run() {
  local watches_file="${MEET_AUTOMATION_WATCHES_PATH:-${SCRAPERS_DIR}/usaw/meet_automation/watches.json}"
  local watch_count
  watch_count="$(python3 -c "import json,sys
try:
    data=json.load(open(sys.argv[1]))
except Exception:
    print(0)
else:
    print(len(data) if isinstance(data, list) else 0)
" "${watches_file}" 2>/dev/null)"
  if [[ "${watch_count}" == "0" ]]; then
    echo "meet automation: no watches in ${watches_file}; skipping."
    return 0
  fi
  meet_automation "$JOB" run --all
}

urlwatch_job() {
  local dir="${SCRAPERS_DIR}/urlwatch"
  local venv="${dir}/.venv"
  if [[ ! -x "${venv}/bin/urlwatch" ]]; then
    python3 -m venv "${venv}"
    "${venv}/bin/python" -m pip install --upgrade pip
    "${venv}/bin/python" -m pip install urlwatch
  fi
  local runtime="${dir}/.runtime"
  mkdir -p "${runtime}/config/urlwatch" "${runtime}/cache"
  sed "s|\${SLACK_URLWATCH_WEBHOOK_URL}|${SLACK_URLWATCH_WEBHOOK_URL:-}|g" "${dir}/urlwatch.yaml" > "${runtime}/config/urlwatch/urlwatch.yaml"

  # urls.yaml is a runtime file: managed live via the Slack /url-* commands
  # (the API edits this same multi-doc YAML) and gitignored so it never collides
  # with the deploy's `git checkout`. Seed it from the committed template the
  # first time, so the built-in watches exist before anything is added.
  local urls_file="${URLWATCH_URLS_PATH:-${dir}/urls.yaml}"
  if [[ ! -s "${urls_file}" && -f "${dir}/urls.example.yaml" ]]; then
    cp "${dir}/urls.example.yaml" "${urls_file}"
  fi
  cp "${urls_file}" "${runtime}/config/urlwatch/urls.yaml"
  XDG_CONFIG_HOME="${runtime}/config" XDG_CACHE_HOME="${runtime}/cache" "${venv}/bin/urlwatch"
}

run_selected_job() {
  case "${JOB}" in
    entries) entry_scrapers ;;
    intl-rankings) python_job "${SCRAPERS_DIR}/usaw/rankings_scraper" intl_rankings_scraper.py --all ;;
    iwf-world-records) python_job "${SCRAPERS_DIR}/iwf/world-records" scraper.py ;;
    meet-automation-approve) meet_automation "$JOB" approve --all-pending ;;
    meet-automation-requests) meet_automation "$JOB" run --requested ;;
    meet-automation-run) meet_automation_run ;;
    meet-sync) meet_sync ;;
    records) python_job "${SCRAPERS_DIR}/usaw/records_scraper" records_scraper.py ;;
    results-sport80) results_sport80 ;;
    standards) python_job "${SCRAPERS_DIR}/usaw/standards_scraper" scraper.py ;;
    umwf-records) python_job "${SCRAPERS_DIR}/usaw/records_scraper" umwf_records.py ;;
    upcoming-meets-slack) node "${SCRAPERS_DIR}/upcoming-meets-slack/slack-upcoming-meets.js" ;;
    urlwatch) urlwatch_job ;;
    usamw-events) usamw_events ;;
    wso-records) wso_scrapers ;;
    *)
      echo >&2 "Unknown scraper job: ${JOB}"
      exit 2
      ;;
  esac
}

if run_selected_job; then
  exit 0
else
  status=$?
  exit "${status}"
fi
