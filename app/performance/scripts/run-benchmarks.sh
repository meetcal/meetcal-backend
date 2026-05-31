#!/usr/bin/env bash
set -euo pipefail

export PATH="${HOME}/.bun/bin:${PATH}"

ROUTE_ID="${1:-meets}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PERF_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
APP_DIR="$(cd "${PERF_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${APP_DIR}/../.." && pwd)"
RESULTS_FILE="${PERF_DIR}/results.md"
ITERATIONS="${ITERATIONS:-25}"
WARMUP="${WARMUP:-1}"
BASE_URL="${BASE_URL:-http://127.0.0.1:3000}"
BENCH_MANAGE_SERVER="${BENCH_MANAGE_SERVER:-1}"

ALL_ROUTES=(meets "meets/:name" "meets/schedule/:name" "meets/athletes/:name" clubs records)

SERVER_PID=""
RUST_PATH=""
RN_SCRIPT=""

# Match real clients: request compressed responses from the Rust API.
CURL_RUST_OPTS=(--compressed -H "Accept-Encoding: gzip, br")

bench_rust_http() {
  local path_url="$1"
  if ! curl -sf "${CURL_RUST_OPTS[@]}" "${BASE_URL}${path_url}" -o /dev/null; then
    echo "error: ${BASE_URL}${path_url} unreachable — start the API with: cd meetcal-backend/app && cargo run --release" >&2
    exit 1
  fi
  for ((i = 0; i < WARMUP; i++)); do
    curl -sf "${CURL_RUST_OPTS[@]}" "${BASE_URL}${path_url}" -o /dev/null
  done
  local times=()
  for ((i = 0; i < ITERATIONS; i++)); do
    local secs
    secs=$(curl -sf "${CURL_RUST_OPTS[@]}" -o /dev/null -w '%{time_total}' "${BASE_URL}${path_url}")
    local ms
    ms=$(awk -v s="$secs" 'BEGIN { printf "%.2f", s * 1000 }')
    times+=("$ms")
  done
  printf '%s\n' "${times[@]}" | sort -n | awk '
    { a[NR] = $1 }
    END {
      n = NR
      if (n == 0) exit 1
      if (n % 2) print a[(n + 1) / 2]
      else print (a[n / 2] + a[n / 2 + 1]) / 2
    }
  '
}

configure_route() {
  local id="$1"
  local meet_name="${BENCH_MEET_NAME:-2026 Ohio WSO Championships}"
  local meet_path_encoded
  meet_path_encoded=$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "${meet_name}")

  case "${id}" in
    meets)
      RUST_PATH="/meets"
      RN_SCRIPT="${REPO_ROOT}/meetcal-app/scripts/performance/bench-meets.ts"
      ;;
    meets/:name)
      RUST_PATH="/meets/${meet_path_encoded}"
      RN_SCRIPT="${REPO_ROOT}/meetcal-app/scripts/performance/bench-meet-details.ts"
      export BENCH_MEET_NAME="${meet_name}"
      ;;
    meets/schedule/:name)
      local schedule_meet_name="${BENCH_SCHEDULE_MEET_NAME:-2026 USA Weightlifting National Championships, Powered by Rogue Fitness}"
      local schedule_path_encoded
      schedule_path_encoded=$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "${schedule_meet_name}")
      RUST_PATH="/meets/schedule/${schedule_path_encoded}"
      RN_SCRIPT="${REPO_ROOT}/meetcal-app/scripts/performance/bench-meet-schedule.ts"
      export BENCH_MEET_NAME="${schedule_meet_name}"
      ;;
    meets/athletes/:name)
      local athletes_meet_name="${BENCH_ATHLETES_MEET_NAME:-2026 USA Weightlifting National Championships, Powered by Rogue Fitness}"
      local athletes_path_encoded
      athletes_path_encoded=$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "${athletes_meet_name}")
      RUST_PATH="/meets/athletes/${athletes_path_encoded}"
      RN_SCRIPT="${REPO_ROOT}/meetcal-app/scripts/performance/bench-meet-athletes.ts"
      export BENCH_MEET_NAME="${athletes_meet_name}"
      ;;
    clubs)
      RUST_PATH="/clubs"
      RN_SCRIPT="${REPO_ROOT}/meetcal-app/scripts/performance/bench-clubs.ts"
      ;;
    records)
      local record_type="${BENCH_RECORD_TYPE:-USAW}"
      local gender="${BENCH_RECORD_GENDER:-men}"
      local age_category="${BENCH_RECORD_AGE_CATEGORY:-senior}"
      RUST_PATH="/records?recordType=${record_type}&gender=${gender}&ageCategory=${age_category}"
      RN_SCRIPT="${REPO_ROOT}/meetcal-app/scripts/performance/bench-records.ts"
      export BENCH_RECORD_TYPE="${record_type}"
      export BENCH_RECORD_GENDER="${gender}"
      export BENCH_RECORD_AGE_CATEGORY="${age_category}"
      ;;
    *)
      echo "unknown route id: ${id}" >&2
      exit 1
      ;;
  esac
}

start_release_server() {
  local health_path="$1"
  if [[ "${BENCH_MANAGE_SERVER}" != "1" ]]; then
    return
  fi
  if [[ -n "${SERVER_PID}" ]]; then
    return
  fi
  fuser -k 3000/tcp 2>/dev/null || true
  sleep 0.5
  echo "starting rust server (release) on ${BASE_URL}..."
  (cd "${APP_DIR}" && cargo run --release) &
  SERVER_PID=$!
  for _ in $(seq 1 60); do
    if curl -sf "${CURL_RUST_OPTS[@]}" "${BASE_URL}${health_path}" -o /dev/null 2>/dev/null; then
      return
    fi
    sleep 1
  done
  echo "rust server (release) failed to start" >&2
  kill "${SERVER_PID}" 2>/dev/null || true
  exit 1
}

cleanup() {
  if [[ -n "${SERVER_PID}" && "${BENCH_MANAGE_SERVER}" == "1" ]]; then
    kill "${SERVER_PID}" 2>/dev/null || true
    fuser -k 3000/tcp 2>/dev/null || true
  fi
}

update_results_md() {
  local route_id="$1"
  local rust_ms="$2"
  local rn_ms="$3"
  local pct="$4"

  python3 - "${RESULTS_FILE}" "${route_id}" "${rust_ms}" "${rn_ms}" "${pct}" <<'PY'
import re
import sys

path, route_id, rust_ms, rn_ms, pct = sys.argv[1:6]
rust_f = float(rust_ms)
rn_f = float(rn_ms)
pct_f = float(pct.rstrip("%"))
delta = abs(rust_f - rn_f)
noticeable = "Yes" if delta >= 100 and abs(pct_f) >= 25 else "No"

TOTAL_LABEL = "**Total** (avg)"
ROUTE_ROW_RE = re.compile(
    r"^\| `GET /[^`]+` \| ([0-9.]+) \| ([0-9.]+) \| (-?[0-9.]+)% \| (Yes|No) \|$"
)


def noticeable_for(rust: float, rn: float, pct: float) -> str:
    delta = abs(rust - rn)
    return "Yes" if delta >= 100 and abs(pct) >= 25 else "No"


def refresh_route_row_noticeable(line: str) -> str:
    match = ROUTE_ROW_RE.match(line)
    if not match:
        return line
    route = line.split("|")[1].strip()
    rust = float(match.group(1))
    rn = float(match.group(2))
    pct = float(match.group(3))
    notice = noticeable_for(rust, rn, pct)
    return f"| {route} | {match.group(1)} | {match.group(2)} | {pct}% | {notice} |"


def is_route_row(line: str) -> bool:
    return line.startswith("| `GET /")


def is_total_row(line: str) -> bool:
    return line.startswith(f"| {TOTAL_LABEL} |")


def parse_route_row(line: str) -> tuple[float, float, float] | None:
    match = ROUTE_ROW_RE.match(line)
    if not match:
        return None
    return float(match.group(1)), float(match.group(2)), float(match.group(3))


def build_total_row(rows: list[tuple[float, float, float]]) -> str:
    n = len(rows)
    avg_rust = sum(r[0] for r in rows) / n
    avg_rn = sum(r[1] for r in rows) / n
    avg_pct = sum(r[2] for r in rows) / n
    notice = noticeable_for(avg_rust, avg_rn, avg_pct)
    return (
        f"| {TOTAL_LABEL} | {avg_rust:.2f} | {avg_rn:.2f} | {avg_pct:.1f}% | {notice} |"
    )


text = open(path).read()

row = f"| `GET /{route_id}` | {rust_ms} | {rn_ms} | {pct}% | {noticeable} |"

lines = text.splitlines()
header_idx = next(
    (i for i, line in enumerate(lines) if line.startswith("| Route |")),
    None,
)
sep_idx = header_idx + 1 if header_idx is not None else None
if header_idx is None:
    data_lines: list[str] = [row]
else:
    data_start = (sep_idx + 1) if sep_idx is not None else header_idx + 1
    data_lines = [
        line for line in lines[data_start:] if is_route_row(line) and not is_total_row(line)
    ]
    replaced = False
    for i, line in enumerate(data_lines):
        if line.startswith(f"| `GET /{route_id}` |"):
            data_lines[i] = row
            replaced = True
            break
    if not replaced:
        data_lines.append(row)

    data_lines = [refresh_route_row_noticeable(line) for line in data_lines]

parsed = [parsed for line in data_lines if (parsed := parse_route_row(line))]
if parsed:
    data_lines.append(build_total_row(parsed))

if header_idx is None:
    lines = [
        "| Route | Rust (ms) | RN client path (ms) | Rust vs RN % | User noticeable |",
        "| --- | ---: | ---: | ---: | :---: |",
        *data_lines,
    ]
else:
    lines = lines[: data_start] + data_lines

open(path, "w").write("\n".join(lines) + "\n")
if parsed:
    avg_pct = sum(r[2] for r in parsed) / len(parsed)
    print(
        f"rust={rust_ms}ms rn={rn_ms}ms change={pct}% noticeable={noticeable} "
        f"total_avg_change={avg_pct:.1f}%"
    )
else:
    print(f"rust={rust_ms}ms rn={rn_ms}ms change={pct}% noticeable={noticeable}")
PY
}

run_route_benchmark() {
  local route_id="$1"
  echo "=== ${route_id} ==="
  echo "benchmarking rust ${RUST_PATH} (release)..."
  local rust_ms
  rust_ms=$(bench_rust_http "${RUST_PATH}")

  echo "benchmarking rn client path..."
  local rn_ms
  rn_ms=$(cd "${REPO_ROOT}/meetcal-app" && ITERATIONS="${ITERATIONS}" WARMUP="${WARMUP}" bun "${RN_SCRIPT}")

  local pct
  pct=$(awk -v r="${rust_ms}" -v n="${rn_ms}" 'BEGIN { printf "%.1f", ((r - n) / n) * 100 }')
  update_results_md "${route_id}" "${rust_ms}" "${rn_ms}" "${pct}"
}

if [[ "${ROUTE_ID}" == "all" ]]; then
  trap cleanup EXIT
  start_release_server "/meets"
  for route in "${ALL_ROUTES[@]}"; do
    configure_route "${route}"
    run_route_benchmark "${route}"
  done
  cleanup
  trap - EXIT
  echo "updated ${RESULTS_FILE} (all routes, release build)"
  exit 0
fi

configure_route "${ROUTE_ID}"
trap cleanup EXIT
start_release_server "${RUST_PATH}"
run_route_benchmark "${ROUTE_ID}"
cleanup
trap - EXIT

echo "updated ${RESULTS_FILE}"
