#!/usr/bin/env bash
set -euo pipefail

ROOT="${MEETCAL_BACKEND_ROOT:-/home/maddisen/dev/meetcal-llc/meetcal-backend}"
ENV_FILE="${ENV_FILE:-${ROOT}/.env}"
IMAGE="${IMAGE:-ghcr.io/meetcal/meetcal-backend/meetcal-api:latest}"
CONTAINER_NAME="${CONTAINER_NAME:-meetcal-api}"
DOCKER_NETWORK="${DOCKER_NETWORK:-meetcal-monitoring}"
APP_DIR="${APP_DIR:-${ROOT}/app}"

if [[ "${BUILD_LOCAL:-0}" == "1" ]]; then
  docker build -t "${IMAGE}" "${APP_DIR}"
elif [[ "${IMAGE}" == ghcr.io/* ]]; then
  docker pull "${IMAGE}"
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

: "${CLERK_JWKS_URL:?CLERK_JWKS_URL must be set in the production env file}"
: "${CLERK_ISSUER:?CLERK_ISSUER must be set in the production env file}"
: "${CLERK_AUTHORIZED_PARTIES:?CLERK_AUTHORIZED_PARTIES must be set in the production env file}"

SCRAPERS_MOUNT="/srv/meetcal-backend/scrapers"
MEET_AUTOMATION_WATCHES_PATH="${MEET_AUTOMATION_WATCHES_PATH:-${SCRAPERS_MOUNT}/usaw/meet_automation/watches.json}"
ENTRIES_TARGETS_PATH="${ENTRIES_TARGETS_PATH:-${SCRAPERS_MOUNT}/usaw/entry_scraper/entries_targets.json}"
MEET_AUTOMATION_STATE_DIR="${MEET_AUTOMATION_STATE_DIR:-${SCRAPERS_MOUNT}/usaw/meet_automation/state}"

env_args=(
  -e APP_APPLICATION_HOST=0.0.0.0
  -e APP_DATABASE__HOST=meetcal
  -e APP_DATABASE__PASSWORD
  -e CLERK_JWKS_URL
  -e CLERK_ISSUER
  -e CLERK_AUTHORIZED_PARTIES
  -e "MEET_AUTOMATION_WATCHES_PATH=${MEET_AUTOMATION_WATCHES_PATH}"
  -e "ENTRIES_TARGETS_PATH=${ENTRIES_TARGETS_PATH}"
  -e "MEET_AUTOMATION_STATE_DIR=${MEET_AUTOMATION_STATE_DIR}"
)

for optional_var in \
  CLERK_AUDIENCE \
  SLACK_SIGNING_SECRET \
  SLACK_MEET_AUTOMATION_CHANNEL \
  SLACK_ENTRIES_CHANNEL \
  MEET_AUTOMATION_SLACK_ALLOWED_USERS; do
  if [[ -n "${!optional_var:-}" ]]; then
    env_args+=(-e "${optional_var}")
  fi
done

docker rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true

# Run as the host user that owns the repo (and the cron jobs), not root, so the
# files the API writes into the bind-mounted state dir (run requests + button
# decision files) are owned by that user. Otherwise the scraper cron — which
# runs as the host user and must delete those files — gets EACCES, the request
# is never consumed, and the job re-runs every tick. Override with API_RUN_USER.
API_RUN_USER="${API_RUN_USER:-$(id -u):$(id -g)}"

docker run -d \
  --name "${CONTAINER_NAME}" \
  --restart unless-stopped \
  --network "${DOCKER_NETWORK}" \
  --user "${API_RUN_USER}" \
  -p 127.0.0.1:3000:3000 \
  -v "${ROOT}/scrapers:${SCRAPERS_MOUNT}" \
  "${env_args[@]}" \
  "${IMAGE}"
