FROM rust:slim AS builder

WORKDIR /usr/src/app

RUN apt-get update \
  && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src

RUN cargo build --release

FROM debian:trixie-slim AS runtime

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /usr/src/app/target/release/app /usr/local/bin/meetcal-api
COPY src/configuration.yaml /app/config/configuration.yaml

ENV APP_APPLICATION_HOST=0.0.0.0
ENV APP_APPLICATION_PORT=3000
ENV APP_CONFIG_DIR=/app/config

EXPOSE 3000

CMD ["meetcal-api"]
