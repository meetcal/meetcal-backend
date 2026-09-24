#!/usr/bin/env bash

load_repo_env() {
  local script_dir
  script_dir="$(cd "$(dirname "${BASH_SOURCE[1]:-${BASH_SOURCE[0]}}")" && pwd)"
  local env_file="${script_dir}/../../.env"
  if [[ -f "${env_file}" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "${env_file}"
    set +a
  fi
}

# Fail unless the database at DATABASE_URL is UTF-8 with a ctype that folds
# Unicode. The API refuses to start otherwise (app/src/common/schema.rs):
# name matching runs lower() and a \s regex in Postgres, and under
# LC_CTYPE=C/POSIX those fold only ASCII, so non-ASCII names would never
# match. LC_CTYPE cannot be changed on an existing database.
check_database_locale() {
  local encoding ctype provider
  encoding="$(psql "${DATABASE_URL}" -Atc "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname = current_database()")"
  ctype="$(psql "${DATABASE_URL}" -Atc "SELECT datctype FROM pg_database WHERE datname = current_database()")"
  provider="$(psql "${DATABASE_URL}" -Atc "SELECT datlocprovider FROM pg_database WHERE datname = current_database()")"
  if [[ "${encoding}" != "UTF8" ]]; then
    echo >&2 "Error: database encoding is ${encoding}; the API requires UTF8."
    echo >&2 "Recreate it: CREATE DATABASE ... ENCODING 'UTF8' LC_CTYPE 'C.UTF-8' LC_COLLATE 'C.UTF-8' TEMPLATE template0"
    return 1
  fi
  if [[ "${provider}" != "i" ]] && { [[ "${ctype}" == "C" ]] || [[ "${ctype}" == "POSIX" ]]; }; then
    echo >&2 "Error: database LC_CTYPE is ${ctype}, which folds only ASCII; the API refuses to start."
    echo >&2 "Recreate it with a UTF-8 ctype, e.g. LC_CTYPE 'C.UTF-8' (or 'en_US.UTF-8'), or the ICU provider."
    return 1
  fi
}
