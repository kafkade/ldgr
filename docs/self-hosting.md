# Self-Hosting ldgr-server (docker-compose)

This is the one-command self-hosting guide for **ldgr-server** — the optional
sync server. It ships as a small docker-compose bundle at the repo root, in the
style of Immich: **copy `docker-compose.yml` + `.env`, run `docker compose up`,
done.**

ldgr-server is a **zero-knowledge encrypted blob relay**. It stores and serves
encrypted blobs but never decrypts them — it never sees your password or your
plaintext financial data. All encryption happens on your devices.

> **Licensing note:** the sync server (`crates/ldgr-server/`) is licensed under
> AGPL-3.0. The rest of ldgr is Apache-2.0.
>
> **New to sync in general?** The
> [Cross-Client Sync Setup guide](sync-setup.md) walks through creating an
> account and registering devices once the server is up. This document is about
> *running the server*.

## Prerequisites

- A host with [Docker](https://docs.docker.com/get-docker/) and the Docker
  Compose plugin (`docker compose version`).
- Optional but recommended for anything internet-facing: a domain name and TLS
  (see [TLS](#tls-with-caddy) below).

## Quick start

```sh
# From a clone of the repo, or a directory containing docker-compose.yml,
# .env.example, and Caddyfile (you can copy just those three files).
cp .env.example .env

# Review the settings — at minimum decide your registration policy and, for a
# personal instance, set an admin bootstrap email.
$EDITOR .env

docker compose up -d
```

Verify it's healthy:

```sh
docker compose ps            # STATUS should show "healthy"
curl -fsS http://localhost:8080/health   # -> "ok"
```

Your server is now listening on `http://<host>:8080`. Point your ldgr clients at
that URL (see the [sync setup guide](sync-setup.md) to create an account and
register devices).

> **Personal instance tip:** the default `LDGR_REGISTRATION=invite-only` is the
> most secure choice but requires an admin-issued invite. For a single-user or
> family instance it's usually easiest to set `LDGR_ADMIN_EMAIL=you@example.com`
> (that account becomes admin on first registration and bypasses the policy), or
> temporarily set `LDGR_REGISTRATION=open` while you register, then switch back.

## Configuration

All configuration is via environment variables in `.env`. The compose-only
variables control the deployment; the `LDGR_*` variables configure the server
itself.

### Compose-only variables

| Variable | Default | Description |
| --- | --- | --- |
| `LDGR_VERSION` | `latest` | GHCR image tag to pull (e.g. `v1.2.3`, `1.2`, `1`, `latest`). Pin an explicit version in production. |
| `LDGR_HOST_PORT` | `8080` | Host port to expose (container always listens on `8080`). |
| `LDGR_DOMAIN` | (unset) | Public domain for the optional Caddy TLS profile. |
| `LDGR_ACME_EMAIL` | (unset) | Email for ACME/Let's Encrypt notices (Caddy TLS profile). |

### Server variables

| Variable | Default | Description |
| --- | --- | --- |
| `LDGR_BIND_ADDR` | `0.0.0.0:8080` | Address the server binds inside the container. Keep `0.0.0.0` so the port is reachable. |
| `LDGR_DB_PATH` | `/data/ldgr-server.db` | SQLite database path (on the `ldgr-data` volume). |
| `LDGR_REGISTRATION` | `invite-only` | Who may register: `open`, `invite-only`, or `admin-only`. |
| `LDGR_ADMIN_EMAIL` | (unset) | Seeds the first admin; that account bypasses the registration policy. |
| `LDGR_SESSION_TTL_HOURS` | `720` | Session lifetime (30 days). |
| `LDGR_RELAY_TTL_MINUTES` | `10` | Key-exchange relay offer TTL. |
| `LDGR_MAX_BLOB_BYTES` | `52428800` | Max blob size (50 MB). |
| `LDGR_SRP_HANDSHAKE_TTL_SECS` | `120` | Login handshake lifetime (seconds). |
| `LDGR_DEFAULT_QUOTA_BYTES` | `1073741824` | Default per-user storage quota (1 GiB). |
| `LDGR_SERVER_NAME` | `ldgr-server` | Cosmetic server name advertised by the discovery endpoint. |

## Building locally instead of pulling

By default compose pulls the published multi-arch image from GHCR. To build from
source instead, edit `docker-compose.yml`: comment out the `image:` line under
the `server` service and uncomment the `build:` block, then:

```sh
docker compose up -d --build
```

## TLS with Caddy

Authentication is zero-knowledge, but the transport still needs TLS for anything
reachable over the internet. The bundle includes an optional
[Caddy](https://caddyserver.com/) reverse proxy that obtains and renews
certificates automatically.

1. Point your domain's DNS `A`/`AAAA` record at the host.
2. In `.env`, set `LDGR_DOMAIN` (and optionally `LDGR_ACME_EMAIL`).
3. Start with the `tls` profile:

   ```sh
   docker compose --profile tls up -d
   ```

Caddy listens on `80`/`443`, terminates TLS, and proxies to the `server`
container. The proxy config lives in `./Caddyfile`. When using the TLS profile
you typically don't need to publish `LDGR_HOST_PORT` to the internet — keep the
server reachable only via the proxy.

### Other proxies (nginx / Traefik)

Any reverse proxy works — just forward to the `server` container's port `8080`
and let the proxy handle TLS. Minimal sketches:

- **nginx:** a `server {}` block with `ssl_certificate*` and
  `location / { proxy_pass http://127.0.0.1:8080; }` (plus your certs, e.g. via
  certbot).
- **Traefik:** attach the `server` service to your Traefik network and add the
  usual `traefik.http.routers.*` labels with a `websecure` entrypoint and a
  cert resolver.

## Upgrading

Images are published to GHCR on each release. To upgrade:

```sh
# Optionally bump LDGR_VERSION in .env to a specific new tag first.
docker compose pull
docker compose up -d
```

Compose recreates the `server` container with the new image; the `ldgr-data`
volume (your database) is preserved. Back up before upgrading (below), and pin
`LDGR_VERSION` so upgrades are deliberate rather than implicit.

## Backup and restore

All server state lives in the named volume `ldgr-data` (the SQLite database at
`/data/ldgr-server.db`).

### Back up

```sh
# Stop the server for a consistent copy, then archive the volume.
docker compose stop server
docker run --rm \
  -v ldgr-data:/data \
  -v "$PWD":/backup \
  busybox tar czf /backup/ldgr-backup-$(date +%Y%m%d).tar.gz -C /data .
docker compose start server
```

This writes `ldgr-backup-YYYYMMDD.tar.gz` to the current directory. Store it
somewhere safe and off-host.

> The volume is named after the compose project (directory name). If your
> project directory isn't the default, run `docker volume ls` to find the exact
> volume name (e.g. `ldgr_ldgr-data`) and use that.

### Restore

```sh
docker compose down
docker volume rm ldgr-data                     # remove the old volume (if present)
docker volume create ldgr-data
docker run --rm \
  -v ldgr-data:/data \
  -v "$PWD":/backup \
  busybox tar xzf /backup/ldgr-backup-YYYYMMDD.tar.gz -C /data
docker compose up -d
```

## Troubleshooting

- **Container is `unhealthy`:** check logs with `docker compose logs -f server`.
  The healthcheck probes `GET /health` inside the container.
- **Can't reach the server:** confirm `LDGR_BIND_ADDR=0.0.0.0:8080` (not
  `127.0.0.1`) and that `LDGR_HOST_PORT` isn't already in use on the host.
- **Registration refused:** that's the default `invite-only` policy. Set
  `LDGR_ADMIN_EMAIL` or `LDGR_REGISTRATION=open` as described in
  [Quick start](#quick-start).
