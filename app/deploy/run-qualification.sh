#!/usr/bin/env bash
# Daily referral qualification + reward minting job.
#
# Calls the API's internal qualification endpoint, which promotes matured
# `qualifying` referrals to `qualified` and mints reward-ledger rows per the
# derived formula (respecting the annual cap). The endpoint is idempotent, so a
# missed or repeated run is safe.
#
# Invoked by cron (see meetcal-scrapers.cron). Requires INTERNAL_JOB_SECRET
# (from meetcal-backend/.env) to match the running API's env.
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../scripts/load_env.sh
source "${SCRIPT_DIR}/../scripts/load_env.sh"
load_repo_env

if [[ -z "${INTERNAL_JOB_SECRET:-}" ]]; then
  echo >&2 "Error: INTERNAL_JOB_SECRET is not set (meetcal-backend/.env)."
  exit 1
fi

API_BASE_URL="${API_BASE_URL:-http://127.0.0.1:3000}"

# Fail the job (non-zero) on any non-2xx so cron mail surfaces it.
http_code="$(curl -sS -o /tmp/run-qualification.out -w '%{http_code}' \
  -X POST "${API_BASE_URL}/internal/referrals/run-qualification" \
  -H "X-Internal-Job-Secret: ${INTERNAL_JOB_SECRET}")"

echo "run-qualification: HTTP ${http_code} $(cat /tmp/run-qualification.out)"
if [[ "${http_code}" != "200" ]]; then
  exit 1
fi
