#!/usr/bin/env bash
set -euo pipefail

export PATH="${HOME}/.bun/bin:${PATH}"

ROUTE_ID="${1:-meets}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PERF_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
APP_DIR="$(cd "${PERF_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${APP_DIR}/../.." && pwd)"
RESULTS_FILE="${PERF_DIR}/results.md"
ITERATIONS="${ITERATIONS:-10}"
WARMUP="${WARMUP:-1}"
BASE_URL="${BASE_URL:-http://127.0.0.1:3000}"

bench_rust_http() {
  local path_url="$1"
  if ! curl -sf "${BASE_URL}${path_url}" -o /dev/null; then
    echo "error: ${BASE_URL}${path_url} unreachable — start the API with: cd meetcal-backend/app && cargo run" >&2
    exit 1
  fi
  for ((i = 0; i < WARMUP; i++)); do
    curl -sf "${BASE_URL}${path_url}" -o /dev/null
  done
  local times=()
  for ((i = 0; i < ITERATIONS; i++)); do
    local secs
    secs=$(curl -sf -o /dev/null -w '%{time_total}' "${BASE_URL}${path_url}")
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

case "${ROUTE_ID}" in
  meets)
    RUST_PATH="/meets"
    RN_SCRIPT="${REPO_ROOT}/meetcal-app/scripts/performance/bench-meets.ts"
    ;;
  *)
    echo "unknown route id: ${ROUTE_ID}" >&2
    exit 1
    ;;
esac

SERVER_PID=""
if ! curl -sf "${BASE_URL}${RUST_PATH}" -o /dev/null 2>/dev/null; then
  echo "starting rust server on ${BASE_URL}..."
  (cd "${APP_DIR}" && cargo run) &
  SERVER_PID=$!
  for _ in $(seq 1 30); do
    if curl -sf "${BASE_URL}${RUST_PATH}" -o /dev/null 2>/dev/null; then
      break
    fi
    sleep 1
  done
  if ! curl -sf "${BASE_URL}${RUST_PATH}" -o /dev/null 2>/dev/null; then
    echo "rust server failed to start" >&2
    kill "${SERVER_PID}" 2>/dev/null || true
    exit 1
  fi
fi

cleanup() {
  if [[ -n "${SERVER_PID}" ]]; then
    kill "${SERVER_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "benchmarking rust ${RUST_PATH}..."
RUST_MS=$(bench_rust_http "${RUST_PATH}")

echo "benchmarking rn client path..."
RN_MS=$(cd "${REPO_ROOT}/meetcal-app" && ITERATIONS="${ITERATIONS}" WARMUP="${WARMUP}" bun "${RN_SCRIPT}")

PCT=$(awk -v r="${RUST_MS}" -v n="${RN_MS}" 'BEGIN { printf "%.1f", ((r - n) / n) * 100 }')

python3 - "${RESULTS_FILE}" "${ROUTE_ID}" "${RUST_MS}" "${RN_MS}" "${PCT}" <<'PY'
import sys

path, route_id, rust_ms, rn_ms, pct = sys.argv[1:6]
rust_f = float(rust_ms)
rn_f = float(rn_ms)
pct_f = float(pct.rstrip("%"))
delta = abs(rust_f - rn_f)
noticeable = "Yes" if delta >= 100 or abs(pct_f) >= 25 else "No"

text = open(path).read()

row = f"| `GET /{route_id}` | {rust_ms} | {rn_ms} | {pct}% | {noticeable} |"

lines = text.splitlines()
header_idx = next(
    (i for i, line in enumerate(lines) if line.startswith("| Route |")),
    None,
)
sep_idx = header_idx + 1 if header_idx is not None else None
if header_idx is None:
    lines = [
        "| Route | Rust (ms) | RN client path (ms) | Rust vs RN % | User noticeable |",
        "| --- | ---: | ---: | ---: | :---: |",
        row,
    ]
else:
    data_start = (sep_idx + 1) if sep_idx is not None else header_idx + 1
    data_lines = lines[data_start:]
    replaced = False
    for i, line in enumerate(data_lines):
        if line.startswith(f"| `GET /{route_id}` |"):
            data_lines[i] = row
            replaced = True
            break
    if not replaced:
        data_lines.append(row)
    lines = lines[:data_start] + data_lines

open(path, "w").write("\n".join(lines) + "\n")
print(f"rust={rust_ms}ms rn={rn_ms}ms change={pct}% noticeable={noticeable}")
PY

cleanup
trap - EXIT

echo "updated ${RESULTS_FILE}"
