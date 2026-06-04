#!/usr/bin/env bash
set -eo pipefail

ENV_FILE="/home/maddisen/dev/meetcal-llc/meetcal-backend/.env"

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

docker build -t meetcal-api .

docker rm -f meetcal-api >/dev/null 2>&1 || true

docker run -d \
  --name meetcal-api \
  --restart unless-stopped \
  --network meetcal-monitoring \
  -p 127.0.0.1:3000:3000 \
  -e APP_APPLICATION_HOST=0.0.0.0 \
  -e APP_DATABASE__HOST=meetcal \
  -e APP_DATABASE__PASSWORD \
  meetcal-api
