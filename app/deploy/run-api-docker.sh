#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export BUILD_LOCAL=1
export IMAGE=meetcal-api
exec bash "${SCRIPT_DIR}/deploy-prod.sh"
