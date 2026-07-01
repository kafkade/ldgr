# ── Builder ────────────────────────────────────────────────────────────────────
FROM rust:1.95-bookworm AS builder

WORKDIR /usr/src/ldgr
COPY . .

RUN cargo build --release -p ldgr-server

# ── Runtime ────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

# ca-certificates for outbound TLS; curl is used by the container/compose
# healthcheck to probe GET /health.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/ldgr/target/release/ldgr-server /usr/local/bin/

# Create the data dir owned by the non-root user so a freshly-created named
# volume mounted at /data is writable (Docker seeds a new volume with the
# ownership of the image's mountpoint).
RUN useradd --create-home --shell /bin/false ldgr \
    && mkdir -p /data \
    && chown ldgr:ldgr /data
USER ldgr

EXPOSE 8080

ENV LDGR_BIND_ADDR=0.0.0.0:8080
ENV LDGR_DB_PATH=/data/ldgr-server.db

VOLUME /data

# Liveness probe for `docker run` (compose overrides with its own healthcheck).
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://localhost:8080/health || exit 1

# OCI image metadata. The server crate is AGPL-3.0 (see crates/ldgr-server/LICENSE).
LABEL org.opencontainers.image.title="ldgr-server" \
      org.opencontainers.image.description="ldgr zero-knowledge encrypted sync server (blob relay)" \
      org.opencontainers.image.source="https://github.com/kafkade/ldgr" \
      org.opencontainers.image.url="https://github.com/kafkade/ldgr" \
      org.opencontainers.image.documentation="https://github.com/kafkade/ldgr/blob/main/docs/self-hosting.md" \
      org.opencontainers.image.licenses="AGPL-3.0-only"

ENTRYPOINT ["ldgr-server"]
