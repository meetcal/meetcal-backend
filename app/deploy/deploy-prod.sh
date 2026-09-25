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

# The audience Clerk puts in the app's session token (`aud: "convex"`, a
# session-token claim kept from the Convex era). Pinned here, after the env
# file is sourced, because a CLERK_AUDIENCE that does not match it answers 401
# to every signed-in /users/me/* request. Change this together with the claim
# in Clerk's session-token settings, Clerk first.
export CLERK_AUDIENCE=convex

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

# Rate limiting (see docs/rate-limits.md). Caddy on the host reaches the
# container through Docker's port publishing, so the API sees the Docker
# network's gateway as its TCP peer, not loopback. Unless the env file sets
# them, trust X-Forwarded-For from loopback and from that gateway only: the
# port is published on 127.0.0.1, so only host processes (Caddy, health
# checks) connect from there, and Caddy overwrites the header.
if [[ -z "${APP_RATE_LIMIT__TRUSTED_PROXIES:-}" ]]; then
  gateways="$(docker network inspect -f '{{range .IPAM.Config}}{{if .Gateway}}{{.Gateway}},{{end}}{{end}}' "${DOCKER_NETWORK}" 2>/dev/null || true)"
  gateways="${gateways%,}"
  if [[ -n "${gateways}" ]]; then
    APP_RATE_LIMIT__TRUSTED_PROXIES="127.0.0.0/8,::1/128,${gateways}"
    export APP_RATE_LIMIT__TRUSTED_PROXIES
  elif [[ "${APP_RATE_LIMIT__ENFORCE:-false}" == "true" ]]; then
    # Without the gateway every visitor would share the proxy's one bucket,
    # and enforcing that would throttle the whole API. Refuse before the
    # running container is touched.
    echo >&2 "Error: APP_RATE_LIMIT__ENFORCE=true but no gateway was found for Docker network ${DOCKER_NETWORK}; set APP_RATE_LIMIT__TRUSTED_PROXIES in the env file."
    exit 1
  else
    echo >&2 "Warning: no gateway found for Docker network ${DOCKER_NETWORK}; X-Forwarded-For will be ignored (shadow mode only logs)."
  fi
fi

for optional_var in \
  APP_RATE_LIMIT__ENFORCE \
  APP_RATE_LIMIT__KEYS \
  APP_RATE_LIMIT__TRUST_FORWARDED_FOR \
  APP_RATE_LIMIT__TRUSTED_PROXIES \
  APP_RATE_LIMIT__IP_TOKENS_PER_SECOND \
  APP_RATE_LIMIT__IP_BURST \
  APP_RATE_LIMIT__KEY_TOKENS_PER_SECOND \
  APP_RATE_LIMIT__KEY_BURST \
  APP_RATE_LIMIT__MAX_IN_FLIGHT \
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
