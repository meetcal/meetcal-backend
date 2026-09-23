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

# The API connects as the least-privileged role, not the postgres superuser:
# row-level security on saved_sessions / user_preferences only applies to
# non-superusers. APP_DATABASE__PASSWORD is that role's password
# (`ALTER ROLE meetcal_api WITH PASSWORD '...'`), not the postgres one.
APP_DATABASE__USERNAME="${APP_DATABASE__USERNAME:-meetcal_api}"
: "${APP_DATABASE__PASSWORD:?APP_DATABASE__PASSWORD (the ${APP_DATABASE__USERNAME} role password) must be set in the production env file}"

env_args=(
  -e APP_APPLICATION_HOST=0.0.0.0
  -e APP_DATABASE__HOST=meetcal
  -e "APP_DATABASE__USERNAME=${APP_DATABASE__USERNAME}"
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

PREVIOUS_NAME="${CONTAINER_NAME}-previous"
HAVE_PREVIOUS=0

# Stop (SIGTERM first, so the API drains in-flight requests via its graceful
# shutdown; `rm -f` alone sends SIGKILL) and keep the old container, renamed,
# with its own image and environment. A failed deploy restarts it untouched,
# which also covers a bad env change such as a wrong database password.
if docker container inspect "${CONTAINER_NAME}" >/dev/null 2>&1; then
  docker stop -t "${STOP_TIMEOUT_SECS:-20}" "${CONTAINER_NAME}" >/dev/null 2>&1 || true
  docker rm -f "${PREVIOUS_NAME}" >/dev/null 2>&1 || true
  docker rename "${CONTAINER_NAME}" "${PREVIOUS_NAME}"
  HAVE_PREVIOUS=1
fi

# Run as the host user that owns the repo (and the cron jobs), not root, so the
# files the API writes into the bind-mounted state dir (run requests + button
# decision files) are owned by that user. Otherwise the scraper cron — which
# runs as the host user and must delete those files — gets EACCES, the request
# is never consumed, and the job re-runs every tick. Override with API_RUN_USER.
API_RUN_USER="${API_RUN_USER:-$(id -u):$(id -g)}"

start_container() {
  docker run -d \
    --name "${CONTAINER_NAME}" \
    --restart unless-stopped \
    --network "${DOCKER_NETWORK}" \
    --user "${API_RUN_USER}" \
    -p 127.0.0.1:3000:3000 \
    -v "${ROOT}/scrapers:${SCRAPERS_MOUNT}" \
    "${env_args[@]}" \
    "$1" >/dev/null
}

# `docker run -d` succeeds even when the API exits at once (wrong database
# password, a migration not yet applied), and `--restart` would then loop it
# while the workflow reports green. Wait for /health instead, and restore the
# previous container if it never answers.
wait_healthy() {
  local deadline=$((SECONDS + ${HEALTH_TIMEOUT_SECS:-60}))
  while ((SECONDS < deadline)); do
    if curl -fsS --max-time 3 "http://127.0.0.1:3000/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  return 1
}

start_container "${IMAGE}"

if wait_healthy; then
  if ((HAVE_PREVIOUS)); then
    docker rm -f "${PREVIOUS_NAME}" >/dev/null 2>&1 || true
  fi
  echo "Deployed ${IMAGE}"
  exit 0
fi

echo >&2 "Error: ${IMAGE} did not become healthy. Last log lines:"
docker logs --tail 40 "${CONTAINER_NAME}" >&2 || true
docker rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true
if ((HAVE_PREVIOUS)); then
  echo >&2 "Restoring the previous container"
  docker rename "${PREVIOUS_NAME}" "${CONTAINER_NAME}"
  docker start "${CONTAINER_NAME}" >/dev/null
  if wait_healthy; then
    echo >&2 "Previous container is serving again; ${IMAGE} was not deployed."
  else
    echo >&2 "Error: the previous container is not healthy either."
  fi
fi
exit 1
