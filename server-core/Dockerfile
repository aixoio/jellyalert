FROM rust:1.98-bookworm AS build
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home jellyalert \
    && mkdir /data \
    && chown jellyalert:jellyalert /data
WORKDIR /app
COPY --from=build /app/target/release/server-core /usr/local/bin/server-core
USER jellyalert
EXPOSE 8090
CMD ["server-core", "/app/server-config.toml"]
