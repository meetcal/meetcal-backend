# Load test

Read-only test of the meet-weekend traffic spike. Run it in a no-meet window.

## 1. Copy the script to the server (from your laptop)

```sh
scp loadtest/meet-weekend.js maddisen@YOUR_SERVER:/tmp/meet-weekend.js
```

## 2. Watch the box (second SSH pane — one command)

```sh
watch -n2 'docker stats --no-stream meetcal-api meetcal; echo; \
  docker exec meetcal psql -U postgres -d meetcal \
  -c "SELECT count(*), state FROM pg_stat_activity GROUP BY state;"'
```

## 3. Run k6 (on the server, via Docker)

Smoke — 100 users:

```sh
docker run --rm -i --network host \
  -e BASE_URL=http://127.0.0.1:3000 \
  -e PEAK_VUS=100 -e PACKAGE_RATE=0.2 \
  -v /tmp/meet-weekend.js:/script.js grafana/k6 run /script.js
```

## 4. Pass/fail (from the k6 summary)

| Metric | Want |
|---|---|
| `http_req_failed` | < 1% |
| `http_req_duration{kind:light}` p95 | < 800ms |
| `package_duration` p95 | < 5s |
