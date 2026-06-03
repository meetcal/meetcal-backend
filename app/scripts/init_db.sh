#!/usr/bin/env bash
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=load_env.sh
source "${SCRIPT_DIR}/load_env.sh"
load_repo_env

if ! command -v psql >/dev/null 2>&1; then
  echo >&2 "Error: psql is not installed."
  exit 1
fi
if ! command -v sqlx >/dev/null 2>&1; then
  echo >&2 "Error: sqlx is not installed."
  echo >&2 "Install with:"
  echo >&2 "  cargo install sqlx-cli --no-default-features --features postgres"
  exit 1
fi

DB_USER="${POSTGRES_USER:=postgres}"
DB_PASSWORD="${POSTGRES_PASSWORD:-${APP_DATABASE__PASSWORD:-}}"
DB_NAME="${POSTGRES_DB:=meetcal}"
DB_PORT="${POSTGRES_PORT:=5432}"
POSTGRES_IMAGE="${POSTGRES_IMAGE:-postgres:16}"

if [[ -z "${DB_PASSWORD}" ]]; then
  echo >&2 "Error: set POSTGRES_PASSWORD or APP_DATABASE__PASSWORD in meetcal-backend/.env"
  exit 1
fi

urlencode() {
  python3 -c 'import urllib.parse, sys; print(urllib.parse.quote(sys.argv[1], safe=""))' "$1"
}

CONTAINER_NAME=meetcal
VOLUME_NAME=meetcal-pgdata

if [[ -z "${SKIP_DOCKER}" ]]; then
  if docker container inspect "${CONTAINER_NAME}" >/dev/null 2>&1; then
    if ! docker inspect -f '{{range .Mounts}}{{if eq .Destination "/var/lib/postgresql/data"}}{{.Name}}{{end}}{{end}}' "${CONTAINER_NAME}" | grep -q .; then
      echo >&2 "Error: ${CONTAINER_NAME} exists without persistent storage."
      echo >&2 "Back up if needed, then: docker rm -f ${CONTAINER_NAME}"
      echo >&2 "Re-run this script to recreate it with volume ${VOLUME_NAME}."
      exit 1
    fi
    docker start "${CONTAINER_NAME}"
  else
    docker volume create "${VOLUME_NAME}" >/dev/null
    docker run \
      --name "${CONTAINER_NAME}" \
      --restart unless-stopped \
      -v "${VOLUME_NAME}:/var/lib/postgresql/data" \
      -e POSTGRES_USER="${DB_USER}" \
      -e POSTGRES_PASSWORD="${DB_PASSWORD}" \
      -e POSTGRES_DB="${DB_NAME}" \
      -p "${DB_PORT}":5432 \
      -d "${POSTGRES_IMAGE}" \
      postgres -N 1000
  fi
fi

export PGPASSWORD="${DB_PASSWORD}"
encoded_password="$(urlencode "${DB_PASSWORD}")"
export DATABASE_URL="${DATABASE_URL:-postgres://${DB_USER}:${encoded_password}@127.0.0.1:${DB_PORT}/${DB_NAME}}"

attempts=0
max_attempts=30
until psql -h 127.0.0.1 -U "${DB_USER}" -p "${DB_PORT}" -d postgres -c '\q' >/dev/null 2>&1; do
  attempts=$((attempts + 1))
  if [[ "${attempts}" -ge "${max_attempts}" ]]; then
    echo >&2 "Error: Postgres did not become ready in time."
    docker logs "${CONTAINER_NAME}" 2>&1 | tail -20 >&2 || true
    exit 1
  fi
  if [[ -n "${SKIP_DOCKER}" ]] || [[ "$(docker inspect -f '{{.State.Status}}' "${CONTAINER_NAME}" 2>/dev/null)" != "running" ]]; then
    echo >&2 "Postgres is still unavailable - sleeping (${attempts}/${max_attempts})"
  fi
  sleep 1
done
echo "Postgres is up and running on port ${DB_PORT} - running migrations now!"
sqlx database create
sqlx migrate run
echo "Postgres has been migrated, ready to go!"
