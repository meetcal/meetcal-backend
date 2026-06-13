#!/usr/bin/env bash
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=load_env.sh
source "${SCRIPT_DIR}/load_env.sh"
load_repo_env

if [[ -z "${DATABASE_URL:-}" ]]; then
  DB_USER="${POSTGRES_USER:-postgres}"
  DB_PASSWORD="${POSTGRES_PASSWORD:-${APP_DATABASE__PASSWORD:-postgres}}"
  DB_HOST="${POSTGRES_HOST:-127.0.0.1}"
  DB_PORT="${POSTGRES_PORT:-5432}"
  DB_NAME="${POSTGRES_DB:-meetcal}"
  DATABASE_URL="postgres://${DB_USER}:${DB_PASSWORD}@${DB_HOST}:${DB_PORT}/${DB_NAME}"
  export DATABASE_URL
fi

if [[ "${MEETCAL_ALLOW_TEST_DB_RESET:-}" != "1" ]]; then
  echo >&2 "Refusing to reset test database without MEETCAL_ALLOW_TEST_DB_RESET=1."
  echo >&2 "This script runs app/scripts/seed_test_db.sql, which truncates app tables."
  exit 1
fi

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

DB_USER="${POSTGRES_USER:-postgres}"
DB_PASSWORD="${POSTGRES_PASSWORD:-${APP_DATABASE__PASSWORD:-postgres}}"
DB_HOST="${POSTGRES_HOST:-127.0.0.1}"
DB_PORT="${POSTGRES_PORT:-5432}"
DB_NAME="${POSTGRES_DB:-meetcal}"
CONTAINER_NAME="${TEST_POSTGRES_CONTAINER:-meetcal-test}"
POSTGRES_IMAGE="${POSTGRES_IMAGE:-postgres:16}"

if [[ -z "${SKIP_DOCKER:-}" ]]; then
  if ! command -v docker >/dev/null 2>&1; then
    echo >&2 "Error: docker is not installed. Set SKIP_DOCKER=1 if Postgres is already running."
    exit 1
  fi
  if ! docker info >/dev/null 2>&1; then
    echo >&2 "Error: docker is not running. Start Docker, or set SKIP_DOCKER=1 if Postgres is already running."
    exit 1
  fi

  if docker container inspect "${CONTAINER_NAME}" >/dev/null 2>&1; then
    docker start "${CONTAINER_NAME}" >/dev/null
  else
    docker run \
      --name "${CONTAINER_NAME}" \
      -e POSTGRES_USER="${DB_USER}" \
      -e POSTGRES_PASSWORD="${DB_PASSWORD}" \
      -e POSTGRES_DB="${DB_NAME}" \
      -p "${DB_PORT}":5432 \
      -d "${POSTGRES_IMAGE}" \
      postgres -N 1000 >/dev/null
  fi
fi

export PGPASSWORD="${DB_PASSWORD}"
attempts=0
max_attempts=30
until psql -h "${DB_HOST}" -U "${DB_USER}" -p "${DB_PORT}" -d postgres -c '\q' >/dev/null 2>&1; do
  attempts=$((attempts + 1))
  if [[ "${attempts}" -ge "${max_attempts}" ]]; then
    echo >&2 "Error: Postgres did not become ready in time."
    docker logs "${CONTAINER_NAME}" 2>&1 | tail -20 >&2 || true
    exit 1
  fi
  sleep 1
done

sqlx database create
(cd "${SCRIPT_DIR}/.." && sqlx migrate run)
psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -f "${SCRIPT_DIR}/seed_test_db.sql"

echo "Test database migrated and seeded."
