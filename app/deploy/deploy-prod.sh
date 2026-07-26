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

SCRAPERS_MOUNT="/srv/meetcal-backend/scrapers"
MEET_AUTOMATION_WATCHES_PATH="${MEET_AUTOMATION_WATCHES_PATH:-${SCRAPERS_MOUNT}/usaw/meet_automation/watches.json}"
ENTRIES_TARGETS_PATH="${ENTRIES_TARGETS_PATH:-${SCRAPERS_MOUNT}/usaw/entry_scraper/entries_targets.json}"
MEET_AUTOMATION_STATE_DIR="${MEET_AUTOMATION_STATE_DIR:-${SCRAPERS_MOUNT}/usaw/meet_automation/state}"

env_args=(
  -e APP_APPLICATION_HOST=0.0.0.0
  -e APP_DATABASE__HOST=meetcal
  -e APP_DATABASE__PASSWORD
  -e "MEET_AUTOMATION_WATCHES_PATH=${MEET_AUTOMATION_WATCHES_PATH}"
  -e "ENTRIES_TARGETS_PATH=${ENTRIES_TARGETS_PATH}"
  -e "MEET_AUTOMATION_STATE_DIR=${MEET_AUTOMATION_STATE_DIR}"
)

for optional_var in \
  SLACK_SIGNING_SECRET \
  SLACK_MEET_AUTOMATION_CHANNEL \
  SLACK_ENTRIES_CHANNEL \
  MEET_AUTOMATION_SLACK_ALLOWED_USERS \
  APP_AUTH__JWKS_URL \
  APP_AUTH__ISSUER \
  APP_AUTH__JWT_VERIFICATION_ENABLED \
  APP_ALLOW_UNVERIFIED_JWT \
  REVENUECAT_WEBHOOK_SECRET \
  INTERNAL_JOB_SECRET \
  REFERRAL_SHARE_BASE_URL \
  REVENUECAT_SECRET_API_KEY \
  APPLE_BUNDLE_ID \
  APPLE_IAP_KEY_ID \
  APPLE_IAP_PRIVATE_KEY \
  APPLE_IAP_OFFER_IDS \
  GOOGLE_PLAY_PACKAGE_NAME \
  GOOGLE_PLAY_SERVICE_ACCOUNT_JSON; do
  if [[ -n "${!optional_var:-}" ]]; then
    env_args+=(-e "${optional_var}")
  fi
done

# Fail closed at deploy time: with JWT verification defaulting to on, the
# container must receive real Clerk settings (via APP_AUTH__JWKS_URL /
# APP_AUTH__ISSUER in the sourced .env), otherwise the API starts up trying to
# fetch JWKS from the configuration.yaml placeholders and every authenticated
# route fails. Catch that here rather than after the container is live.
if [[ -z "${APP_AUTH__JWKS_URL:-}" || -z "${APP_AUTH__ISSUER:-}" ]] \
  && [[ "${APP_ALLOW_UNVERIFIED_JWT:-}" != "1" ]]; then
  echo >&2 "Error: APP_AUTH__JWKS_URL and APP_AUTH__ISSUER must be set in ${ENV_FILE}"
  echo >&2 "       (Clerk JWKS + issuer). Set APP_ALLOW_UNVERIFIED_JWT=1 only for"
  echo >&2 "       explicitly unauthenticated local/testing deploys."
  exit 1
fi

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
