#!/usr/bin/env bash
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=load_env.sh
source "${SCRIPT_DIR}/load_env.sh"
load_repo_env
BACKUP_DIR="${BACKUP_DIR:-${SCRIPT_DIR}/../backups}"
CONTAINER_NAME="${CONTAINER_NAME:-meetcal}"
DB_USER="${POSTGRES_USER:-postgres}"
DB_NAME="${POSTGRES_DB:-meetcal}"
KEEP_DAYS="${KEEP_DAYS:-14}"

mkdir -p "${BACKUP_DIR}"

timestamp="$(date -u +%Y%m%d_%H%M%S)"
output="${BACKUP_DIR}/${DB_NAME}_${timestamp}.sql.gz"

docker exec "${CONTAINER_NAME}" pg_dump -U "${DB_USER}" "${DB_NAME}" | gzip > "${output}"
echo "Wrote ${output}"

find "${BACKUP_DIR}" -name "${DB_NAME}_*.sql.gz" -type f -mtime +"${KEEP_DAYS}" -delete
