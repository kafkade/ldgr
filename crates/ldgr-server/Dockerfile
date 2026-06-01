# ── Builder ────────────────────────────────────────────────────────────────────
FROM rust:1.88-bookworm AS builder

WORKDIR /usr/src/ldgr
COPY . .

RUN cargo build --release -p ldgr-server

# ── Runtime ────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/ldgr/target/release/ldgr-server /usr/local/bin/

RUN useradd --create-home --shell /bin/false ldgr
USER ldgr

EXPOSE 8080

ENV LDGR_BIND_ADDR=0.0.0.0:8080
ENV LDGR_DB_PATH=/data/ldgr-server.db

VOLUME /data

ENTRYPOINT ["ldgr-server"]
