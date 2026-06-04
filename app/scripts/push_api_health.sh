#!/usr/bin/env bash
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=load_env.sh
source "${SCRIPT_DIR}/load_env.sh"
load_repo_env

API_HEALTH_URL="${API_HEALTH_URL:-http://127.0.0.1:3000/meets}"

url_joiner() {
  if [[ "$1" == *\?* ]]; then
    printf '&'
  else
    printf '?'
  fi
}

notify_uptime_kuma() {
  local status="$1"
  local msg="$2"
  local push_url

  if [[ -z "${UPTIME_KUMA_API_PUSH_URL:-}" ]] || ! command -v curl >/dev/null 2>&1; then
    return
  fi

  push_url="${UPTIME_KUMA_API_PUSH_URL%%\?*}"

  curl -fsS --max-time 10 \
    "${push_url}$(url_joiner "${push_url}")status=${status}&msg=${msg}" \
    >/dev/null || true
}

if curl -fsS --max-time 10 "${API_HEALTH_URL}" >/dev/null; then
  notify_uptime_kuma up api_ok
else
  notify_uptime_kuma down api_down
  exit 1
fi
