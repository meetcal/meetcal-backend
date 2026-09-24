# Pinned to the stable minor that CI builds with (dtolnay/rust-toolchain@stable
# was 1.94 at the time of writing); bump it deliberately, alongside the
# toolchain, rather than letting an unpinned tag drift.
FROM rust:1.94-slim AS builder

WORKDIR /usr/src/app

RUN apt-get update \
  && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src

# --locked: the image must build from the committed Cargo.lock, exactly as CI
# tests it, never from a freshly resolved dependency set.
RUN cargo build --release --locked

FROM debian:trixie-slim AS runtime

# curl is only for the HEALTHCHECK below.
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /usr/src/app/target/release/app /usr/local/bin/meetcal-api
COPY src/configuration.yaml /app/config/configuration.yaml

ENV APP_APPLICATION_HOST=0.0.0.0
ENV APP_APPLICATION_PORT=3000
ENV APP_CONFIG_DIR=/app/config

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:${APP_APPLICATION_PORT}/health" || exit 1

CMD ["meetcal-api"]
