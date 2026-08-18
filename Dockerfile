# Multi-stage Docker build for ChocoBase Server Daemon (chocod)
FROM rust:1.80-slim-bullseye AS builder

WORKDIR /usr/src/chocobase
COPY . .

RUN cargo build --release --bin chocod

# Minimal runtime image
FROM debian:bullseye-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

RUN groupadd -g 10001 chocobase && \
    useradd -u 10001 -g chocobase -m -d /app -s /bin/false chocobase

WORKDIR /app
COPY --from=builder /usr/src/chocobase/target/release/chocod /app/chocod

ENV CHOCOBASE_DATA_DIR=/data
RUN mkdir -p /data && chown -R chocobase:chocobase /data /app

USER chocobase:chocobase

EXPOSE 5433 8080

HEALTHCHECK --interval=10s --timeout=3s --retries=3 \
  CMD curl -f http://localhost:8080/v1/health || exit 1

ENTRYPOINT ["/app/chocod", "/data/chocobase.db"]
