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
KEEP_DAYS="${KEEP_DAYS:-30}"
R2_PREFIX="${R2_PREFIX:-postgres}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo >&2 "Error: $1 is not installed."
    exit 1
  fi
}

require_command docker
require_command gzip

mkdir -p "${BACKUP_DIR}"
BACKUP_DIR="$(cd "${BACKUP_DIR}" && pwd)"

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

  if [[ -z "${UPTIME_KUMA_PUSH_URL:-}" ]] || ! command -v curl >/dev/null 2>&1; then
    return
  fi

  push_url="${UPTIME_KUMA_PUSH_URL%%\?*}"

  curl -fsS --max-time 10 \
    "${push_url}$(url_joiner "${push_url}")status=${status}&msg=${msg}" \
    >/dev/null || true
}

on_exit() {
  local exit_code="$1"

  if [[ "${exit_code}" -eq 0 ]]; then
    notify_uptime_kuma up backup_ok
  else
    notify_uptime_kuma down backup_failed
  fi
}
trap 'on_exit "$?"' EXIT

aws_cli() {
  if command -v aws >/dev/null 2>&1; then
    aws "$@"
    return
  fi

  docker run --rm \
    -e AWS_ACCESS_KEY_ID \
    -e AWS_SECRET_ACCESS_KEY \
    -e AWS_DEFAULT_REGION \
    -v "${BACKUP_DIR}:${BACKUP_DIR}:ro" \
    amazon/aws-cli "$@"
}

timestamp="$(date -u +%Y%m%d_%H%M%S)"
output="${BACKUP_DIR}/${DB_NAME}_${timestamp}.sql.gz"
backup_name="$(basename "${output}")"

docker exec "${CONTAINER_NAME}" pg_dump -U "${DB_USER}" "${DB_NAME}" | gzip > "${output}"
echo "Wrote ${output}"

if [[ -n "${R2_BUCKET:-}" ]]; then
  if [[ -z "${R2_ENDPOINT_URL:-}" ]]; then
    if [[ -z "${R2_ACCOUNT_ID:-}" ]]; then
      echo >&2 "Error: set R2_ENDPOINT_URL or R2_ACCOUNT_ID when R2_BUCKET is set."
      exit 1
    fi
    R2_ENDPOINT_URL="https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com"
  fi

  export AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-auto}"
  r2_uri="s3://${R2_BUCKET}/${R2_PREFIX%/}/${backup_name}"

  aws_cli s3 cp "${output}" "${r2_uri}" --endpoint-url "${R2_ENDPOINT_URL}"
  echo "Uploaded ${r2_uri}"

  cutoff_epoch="$(date -u -d "${KEEP_DAYS} days ago" +%s)"
  aws_cli s3 ls "s3://${R2_BUCKET}/${R2_PREFIX%/}/" \
    --recursive \
    --endpoint-url "${R2_ENDPOINT_URL}" |
    while read -r object_date object_time _ object_key; do
      [[ -z "${object_key:-}" ]] && continue
      object_epoch="$(date -u -d "${object_date} ${object_time}" +%s)"
      if [[ "${object_epoch}" -lt "${cutoff_epoch}" ]]; then
        aws_cli s3 rm "s3://${R2_BUCKET}/${object_key}" --endpoint-url "${R2_ENDPOINT_URL}"
        echo "Deleted s3://${R2_BUCKET}/${object_key}"
      fi
    done
fi

find "${BACKUP_DIR}" -name "${DB_NAME}_*.sql.gz" -type f -mtime +"${KEEP_DAYS}" -delete
