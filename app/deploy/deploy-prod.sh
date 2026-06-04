#!/usr/bin/env bash
set -euo pipefail

ROOT="${MEETCAL_BACKEND_ROOT:-/home/maddisen/dev/meetcal-llc/meetcal-backend}"
ENV_FILE="${ENV_FILE:-${ROOT}/.env}"
IMAGE="${IMAGE:-ghcr.io/memohnsen/meetcal-backend/meetcal-api:latest}"
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

docker rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true

docker run -d \
  --name "${CONTAINER_NAME}" \
  --restart unless-stopped \
  --network "${DOCKER_NETWORK}" \
  -p 127.0.0.1:3000:3000 \
  -e APP_APPLICATION_HOST=0.0.0.0 \
  -e APP_DATABASE__HOST=meetcal \
  -e APP_DATABASE__PASSWORD \
  "${IMAGE}"
