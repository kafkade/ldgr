# syntax=docker/dockerfile:1
#
# ldgr-web — static admin panel + web client, served by Caddy.
#
# The Apache-2.0 web app is a Next.js static export bundled with the in-browser
# WASM client. This image builds the export and serves it from Caddy, reverse-
# proxying the JSON API to the ldgr-server container so the panel is reachable
# out of the box (`docker compose up`). The AGPL ldgr-server never serves these
# assets (ADR-006 licensing split; ADR-008 §7).
#
# Build context is the repository root (the WASM crate lives outside apps/web):
#   docker build -f apps/web/Dockerfile -t ldgr-web .

# ── WASM builder ────────────────────────────────────────────────────────────────
# Pinned to the same Rust as crates/ldgr-server/Dockerfile (workspace MSRV 1.95).
FROM rust:1.95-bookworm AS wasm
RUN rustup target add wasm32-unknown-unknown \
    && curl -sSf https://rustwasm.github.io/wasm-pack/installer/init.sh | sh
WORKDIR /src
COPY . .
# Mirrors apps/web `npm run build:wasm` (core + sync features only).
RUN wasm-pack build crates/ldgr-wasm --target web --release \
        --out-dir /src/apps/web/pkg -- --no-default-features --features core,sync

# ── Web builder ─────────────────────────────────────────────────────────────────
FROM node:22-bookworm AS web
WORKDIR /src/apps/web
# Install deps first for layer caching.
COPY apps/web/package.json apps/web/package-lock.json ./
RUN npm ci
# App sources + the WASM package produced above.
COPY apps/web/ ./
COPY --from=wasm /src/apps/web/pkg ./pkg
# The sync engine loads sql.js at runtime; ship its wasm alongside the export.
RUN mkdir -p public/sql.js \
    && cp node_modules/sql.js/dist/sql-wasm.wasm public/sql.js/ \
    && npm run build

# ── Runtime (Caddy static server) ───────────────────────────────────────────────
FROM caddy:2
COPY apps/web/Caddyfile /etc/caddy/Caddyfile
COPY --from=web /src/apps/web/out /srv/www

EXPOSE 8080

# Server API endpoint Caddy proxies to (overridable for non-compose setups).
ENV LDGR_API_UPSTREAM=server:8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD wget -qO- http://localhost:8080/ >/dev/null 2>&1 || exit 1

# OCI image metadata. The web app is Apache-2.0 (see LICENSE at the repo root).
LABEL org.opencontainers.image.title="ldgr-web" \
      org.opencontainers.image.description="ldgr web app + admin panel (static export served by Caddy)" \
      org.opencontainers.image.source="https://github.com/kafkade/ldgr" \
      org.opencontainers.image.url="https://github.com/kafkade/ldgr" \
      org.opencontainers.image.documentation="https://github.com/kafkade/ldgr/blob/main/docs/self-hosting.md" \
      org.opencontainers.image.licenses="Apache-2.0"
