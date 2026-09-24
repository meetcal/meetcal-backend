# Rate limits and load shedding: deploy notes

The API has two layers of abuse protection. Code: `app/src/common/rate_limit.rs` and `app/src/common/load_shed.rs`.

1. **Per-client token buckets.** Anonymous clients are keyed by address (IPv4, or IPv6 `/64`). Clients with a valid `X-MeetCal-Key` are keyed by key name. Expensive routes cost more tokens. Past the limit the API answers `429 {"error":"rate limited"}` with `Retry-After`, **but only when `APP_RATE_LIMIT__ENFORCE=true`**. Until then it runs in shadow mode: nothing is rejected, and a would-be rejection logs one warning per client per minute.
2. **Global in-flight cap.** At most `max_in_flight` requests (default 500) run at once. Past that the API answers `503 {"error":"overloaded"}` with `Retry-After: 1` right away, instead of queueing on the Postgres pool. This is **always on**, whatever `ENFORCE` says.

`/health` is exempt from both.

## Production `.env`

Nothing is required: a deploy without any of these runs in shadow mode with the defaults. `deploy-prod.sh` passes each of these into the container when it is set.

| Variable | Set it to | Notes |
| --- | --- | --- |
| `APP_RATE_LIMIT__KEYS` | `atlas:<secret>` | Named API keys, comma-separated `name:secret`. Generate each secret with `openssl rand -hex 32`, then put the same value in atlas-rn's Convex environment, sent as `X-MeetCal-Key`. Names are `a-z 0-9 - _`. Only SHA-256 digests are kept in memory. Environment only, never `configuration.yaml`. A malformed value stops startup, and the deploy then keeps the previous container. |
| `APP_RATE_LIMIT__ENFORCE` | leave unset (`false`), then `true` | See the rollout below. |
| `APP_RATE_LIMIT__TRUSTED_PROXIES` | leave unset | When unset, `deploy-prod.sh` sets it to `127.0.0.0/8,::1/128,<gateway of the meetcal-monitoring Docker network>`. Set it yourself only if Caddy moves, for example into a container, in which case you list that container's address. |
| `APP_RATE_LIMIT__TRUST_FORWARDED_FOR` | leave unset (`true`) | `false` ignores `X-Forwarded-For` entirely. Behind Caddy, that would put every visitor in one bucket. |
| `APP_RATE_LIMIT__IP_TOKENS_PER_SECOND` / `__IP_BURST` | leave unset (40 / 1200) | Raise these if shadow logs show a venue address being limited on a meet day. |
| `APP_RATE_LIMIT__KEY_TOKENS_PER_SECOND` / `__KEY_BURST` | leave unset (200 / 6000) | Shared by every key. |
| `APP_RATE_LIMIT__MAX_IN_FLIGHT` | leave unset (500) | See `DEFAULT_MAX_IN_FLIGHT` in `app/src/lib.rs` for the sizing. Raise it together with `MAX_DB_CONNECTIONS`, not on its own. |

### Why the Docker gateway is trusted

The per-client limit needs each visitor's address. Caddy overwrites `X-Forwarded-For` with the address it accepted the connection from, so the API trusts the rightmost entry, but only from a peer it knows to be the proxy. On this host Caddy reaches the container through `-p 127.0.0.1:3000:3000`. Docker relays that connection, so the container sees it coming from the Docker network's gateway (for example `172.18.0.1`), not from loopback.

The port is published on `127.0.0.1` only, so only host processes can connect from that address: Caddy, the deploy health check, and cron. Trusting the gateway is therefore safe. If the gateway is missing from the trusted list, every visitor shares Caddy's one bucket. `deploy-prod.sh` refuses to deploy with `ENFORCE=true` when it cannot find the gateway, and the API logs a warning (below).

## Rollout

1. **Deploy in shadow mode.** Add `APP_RATE_LIMIT__KEYS` if atlas-rn is ready, and leave `ENFORCE` unset. Merge, or run `app/deploy/deploy-prod.sh`.
2. **Check the startup line.**

   ```sh
   docker logs meetcal-api 2>&1 | grep 'rate limiting configured'
   ```

   Confirm `enforce=false`, that `trusted_proxies` includes the gateway, and that `api_keys=` lists the expected names (names only, never secrets).
3. **Check client identity.** This warning must not appear:

   ```sh
   docker logs meetcal-api 2>&1 | grep 'ignored X-Forwarded-For'
   ```

   If it does, the `peer=` field shows the proxy address to add to `APP_RATE_LIMIT__TRUSTED_PROXIES`.
4. **Watch real traffic for a few days, including a meet weekend if possible.**

   ```sh
   docker logs --since 72h meetcal-api 2>&1 | grep 'rate limit exceeded' \
     | grep -o 'bucket=[^ ]* client_tag=[^ ]* path=[^ ]*' | sort | uniq -c | sort -rn | head -20
   ```

   Each line is at most one per client per minute. It carries the bucket (`ip` or `key:<name>`), a short `client_tag` (a keyed hash of the address, random per process, so tags reset on restart; the address itself is never logged), and the path, never the query.
   - A tag that appears minute after minute on non-meet days is a host that hammers the API. Enforcing will throttle it, which is the point.
   - A tag that shows up only on a meet day, across ordinary app paths (`/meets`, `/meets/package`, `/meets/schedule`), is probably a venue network. Raise `APP_RATE_LIMIT__IP_TOKENS_PER_SECOND` and `APP_RATE_LIMIT__IP_BURST` before enforcing.
   - A `key:<name>` bucket showing up means that partner needs a bigger key budget.

   Also watch `in-flight request cap reached`. That one is already enforced. If it appears outside an attack, the database is the bottleneck.
5. **Enforce.** Set `APP_RATE_LIMIT__ENFORCE="true"` in `.env` and redeploy. The startup line then shows `enforce=true`. The same warnings keep coming, now with `enforced=true`, and those requests get `429`.
6. **Roll back** by setting `APP_RATE_LIMIT__ENFORCE="false"` and redeploying. No code change is needed.

## Client behaviour

- meetcal-cli retries `429`/`503` twice, honouring `Retry-After` (clamped to 1–30s).
- meetcal-web reads `Retry-After` cross-origin, because it is in `Access-Control-Expose-Headers` and CORS wraps both the `429` and the `503`.
- API keys are for server-side callers only. `X-MeetCal-Key` is deliberately not an allowed CORS request header.
