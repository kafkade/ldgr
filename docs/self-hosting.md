# Self-Hosting ldgr

This is the end-to-end self-hosting guide for **ldgr-server** — the optional sync
server. It takes you all the way from a blank host to a running instance with an
admin, a second user, and clients syncing across devices, in the style of Immich:
**copy `docker-compose.yml` + `.env`, run `docker compose up`, done** — then
onboard as admin and add users from the web admin panel.

ldgr-server is a **zero-knowledge encrypted blob relay**. It stores and serves
encrypted blobs but never decrypts them — it never sees your password or your
plaintext financial data. All encryption happens on your devices.

> **Licensing note:** the sync server (`crates/ldgr-server/`) is licensed under
> AGPL-3.0. The rest of ldgr is Apache-2.0.

## This guide in order

Follow it top to bottom the first time:

1. **[Deploy the server](#quick-start)** — `docker compose up`, then keep it
   running (config, TLS, upgrades, backups).
2. **[Onboard as admin](#first-run-admin-onboarding)** — bootstrap the first
   admin account and open the web admin panel.
3. **[Choose a registration policy](#registration-policy)** — open, invite-only,
   or admin-only.
4. **[Add users](#adding-users)** — invite people and manage accounts, roles, and
   quotas.
5. **[Understand the account model](#the-two-secret-account-model)** — master
   password, vault recovery key, and account Secret Key + Emergency Kit.
6. **[Point a client at your server](#point-a-client-at-your-server) and add a
   device** — sign in with the two-secret model.
7. **[Review the threat model](#security--threat-model)** — what the server can
   and cannot see.

> **Just want the client walkthrough?** Once the server is up, the
> [Cross-Client Sync Setup guide](sync-setup.md) is the deep, click-by-click tour
> of creating an account, registering each device, syncing a transaction, and
> resolving conflicts across the Web, iOS/macOS, and CLI apps. This document is
> the operator's guide: *running the server and onboarding people onto it*.

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

## First-run admin onboarding

The server is headless — it exposes only a JSON API. The **admin experience lives
in the web app** at `/admin` (Apache-2.0), which talks to the server's
`/api/v1/admin/*` API over HTTP (see [ADR-008 §7](adr/008-self-hosting-and-account-auth.md)).
The server itself serves no admin HTML.

### 1. Create the bootstrap admin

The **first account** to register on a fresh server becomes an `admin` and
bypasses the registration policy — even under the default `invite-only`. You have
two ways to establish it:

- **Env-seeded (recommended for unattended deploys):** set
  `LDGR_ADMIN_EMAIL=you@example.com` in `.env` before first boot. The account that
  registers with that identity becomes the bootstrap admin.
- **First-come:** leave `LDGR_ADMIN_EMAIL` unset and simply register the first
  account from any client — it is promoted to admin automatically.

Register that first account from any ldgr client (Web, iOS/macOS, or CLI) pointed
at your server URL — see [Point a client at your server](#point-a-client-at-your-server).

### 2. Open the admin panel

The admin panel ships with the ldgr **web app**. Run the web app (or use your
hosted deployment of it), then browse to `/admin`:

- Enter your **Server URL**, **admin username**, and **password**.
- Sign-in runs the same **SRP handshake** as any client via the in-browser WASM
  client: your password is processed locally and only a zero-knowledge proof is
  sent. After the handshake the panel checks that your account is an admin;
  non-admin accounts are refused.

The panel has five screens:

| Screen | What it does |
| --- | --- |
| **Users** | List accounts; change role, set/clear per-user quota, disable/enable, delete. |
| **Invites** | Issue, list, and revoke invite tokens. |
| **Settings** | Registration policy, default quota, max blob size. |
| **Storage** | Per-user and total storage usage. |
| **Server** | Server name, version, and protocol info. |

> The admin session token is held in memory / `sessionStorage` only (never in a
> vault or `localStorage`) and is cleared on sign-out. Building the web app's WASM
> bundle (`npm run build:wasm`) is required because sign-in uses the SRP client.

## Registration policy

The registration policy decides **who may create new accounts**. Set it with the
`LDGR_REGISTRATION` environment variable, or change it live from the admin
**Settings** screen.

| Policy | Behaviour |
| --- | --- |
| `open` | Anyone can self-register. Best for a personal or family instance. |
| `invite-only` (default) | New accounts require an admin-issued invite token. |
| `admin-only` | Public self-registration is refused entirely. |

Regardless of policy, **the first account on an empty server always succeeds** and
becomes the admin (see [admin onboarding](#first-run-admin-onboarding)).

Which to pick:

- **Personal / single-user** across your own devices: any policy works — your
  first device registers, every other device just *signs in* with the same
  account, so the policy never blocks you.
- **Family / small group:** `open` is the least friction (everyone self-registers),
  or `invite-only` if you want to gate who joins.
- **Public / internet-exposed:** keep `invite-only` (default) or `admin-only` so
  strangers can't create accounts on your instance.

## Adding users

A second, distinct account (e.g. a partner with their own login) is exactly what
`invite-only` gates. There are two supported paths.

### The easy path today: open registration

The simplest way to add people right now is to set `LDGR_REGISTRATION=open` (in
`.env` or the admin **Settings** screen) and have each person self-register from
their own client, then switch the policy back if you like. This works end-to-end
in every client today.

### The invite flow

Under `invite-only`, add a user by issuing an invite from the admin **Invites**
screen:

1. In **Invites**, optionally set the invitee's **email** and **role**
   (`user`/`admin`) and an optional **expiry (hours)**, then **Create invite**.
2. The panel shows the raw **invite token exactly once** — copy it and share it
   with the invitee over a trusted channel.
3. The invitee redeems the token when they register their account.

> **Current limitation.** The admin panel and the server API (`POST
> /api/v1/admin/invites`, redeemed via the `invite_token` field on register) fully
> support invites, but the **client sign-up screens (Web/CLI) do not yet expose an
> invite-token field**. So under `invite-only` an invitee can't redeem a token
> through the normal client UI today. Until that lands, use **open registration**
> (above) to add users, or drive the register API with the token directly. Track
> this gap before relying on `invite-only` for onboarding new people.

### Managing existing users

From the admin **Users** screen you can, for any account:

- **Change role** between `user` and `admin`.
- **Set or clear a per-user storage quota** (clearing falls back to the server
  default, `LDGR_DEFAULT_QUOTA_BYTES`).
- **Disable / enable** the account (a disabled account can't sign in).
- **Delete** the account.

**Last-admin protection:** the server refuses to demote, disable, or delete the
final active admin, so you can't lock yourself out of the instance.

## The two-secret account model

ldgr uses a **1Password-style two-secret model** for server sign-in, layered on
top of its existing local-first vault encryption. The full rationale is in
[ADR-008](adr/008-self-hosting-and-account-auth.md); here's what an operator and
their users need to know.

There are **three distinct secrets**, and it's important not to confuse them:

| Secret | What it protects | Needed when | If lost |
| --- | --- | --- | --- |
| **Master password** | Opens your vault (derives the encryption keys) **and** is one of the two server sign-in factors | Every vault unlock; every server sign-in | Open the vault with the **vault recovery key**; reset server credentials via account recovery. Losing password *and* recovery key = unrecoverable vault (zero-knowledge). |
| **Vault recovery key** (52-character Crockford Base32 string, generated at vault creation) | Unwraps your vault **offline** if you forget the password | Emergency **local** vault access only | The vault still opens with the password. Losing **both** = permanent local data loss. No server involvement. |
| **Account Secret Key** (`A1-…`, generated at sign-up) | A second server sign-in factor mixed into the SRP verifier — the server never receives it | **Signing in on a new device** (and at registration) | You can keep using devices already signed in, but can't add new ones until you recover the account. **Your local vault is unaffected** — it still opens offline with the password. |

Two independent trust domains: the **vault recovery key** gates *plaintext data*;
the **account Secret Key** gates *server access*. Keep them separate — one stolen
sheet should never compromise both.

### The Emergency Kit

At sign-up on a two-secret server, the client generates your **Account Secret
Key** and shows it **once** inside a printable/QR **Emergency Kit** containing:

| Emergency Kit contains | |
| --- | --- |
| Sign-in address | Your server URL (e.g. `https://ledger.example.org`) |
| Account identity | Email / username |
| Account Secret Key | `A1-…` |
| QR payload | The three items above, for fast new-device sign-in |

**Save it before continuing** (password manager, print, or download). The
Emergency Kit deliberately does **not** include your vault recovery key — that
stays a separate artifact for the reasons above.

### The Secret Key format

The Account Secret Key is a versioned, human-transcribable string:

```
A1-7QK2R9-XJ4F NK8H 2W6P ... M3VT 9B0C     (spaces/dashes ignored on decode)
└┬┘ └──┬─┘ └───────────── ≥128-bit random body ─────────────┘
 │     └ account-id hint (not a secret; helps pair a Kit with the right account)
 └ version prefix: 'A' = ldgr account key, '1' = scheme version
```

It is intentionally distinct from the vault recovery key (a bare Crockford string
with no `A1-` prefix), so the two artifacts are never confusable.

## Point a client at your server

Any ldgr client connects the same way: **enter the server URL**, then create an
account (first device) or sign in (later devices).

- **First device — Create Account.** On a two-secret server the client generates
  your **Account Secret Key** and shows the **Emergency Kit** once. Save it.
- **New device — Sign In.** The client asks for your **email + master password +
  Account Secret Key** (typed, or scanned from the Kit's QR). After that the device
  remembers the Secret Key, so future sign-ins on that device need only the
  password.
- **Vault decryption is a separate gate.** Signing in downloads encrypted blobs;
  you still need your **master password** to decrypt them locally. Server auth and
  vault decryption stay independent.

Servers that don't advertise two-secret auth fall back to single-secret
(password-only) SRP-6a automatically.

Per-client entry points:

| Client | Where |
| --- | --- |
| **Web** | Vault **Settings → Sync (ldgr-server)** → enter URL → **Connect**, then **Create Account** / **Sign In**. |
| **iOS / iPadOS / macOS** | **Sync** settings → **Connect**, then **Create Account** / **Sign In on This Device**. |
| **CLI** | `ldgr sync setup` (interactive), then `ldgr sync push` / `pull` / `status`. |

For the full click-by-click walkthrough — registering each device, syncing a
transaction between two clients, and resolving conflicts — follow the
[Cross-Client Sync Setup guide](sync-setup.md).

## Security & threat model

For the full rationale see
[ADR-008](adr/008-self-hosting-and-account-auth.md). In short:

**What the server holds** (nothing financial, nothing that reveals a password):

| The server holds | Sensitive? |
| --- | --- |
| Encrypted vault blobs | No — AES-256-GCM ciphertext, size-bucket padded |
| `(salt, verifier)` per account | No — the verifier is `g^x mod N`, not your password |
| Email / username | Identity only |

**What the server never sees:** your password, your Account Secret Key, your
encryption keys, or any plaintext financial data. With SRP-6a your password is
never transmitted during sign-in — the server only checks a zero-knowledge proof
against the stored verifier.

**Offline brute-force resistance.** If an attacker steals the entire server
database:

| Scheme | What protects the verifier |
| --- | --- |
| **Single-secret** (legacy password-only SRP) | Only the password's entropy + Argon2id cost. A weak password is brute-forceable offline. |
| **Two-secret** (2SKD) | The ≥128-bit **Account Secret Key** — never sent to the server — is mixed into the verifier. Even with the full DB and a weak password, the verifier is computationally useless without the Secret Key. |

The Secret Key is the difference between "password-strength security" and
"≥128-bit security" for server auth. It does **not** affect vault-at-rest
security, which is governed independently by the password + Argon2id +
AES-256-GCM envelope.

## Troubleshooting

- **Container is `unhealthy`:** check logs with `docker compose logs -f server`.
  The healthcheck probes `GET /health` inside the container.
- **Can't reach the server:** confirm `LDGR_BIND_ADDR=0.0.0.0:8080` (not
  `127.0.0.1`) and that `LDGR_HOST_PORT` isn't already in use on the host.
- **Registration refused:** that's the default `invite-only` policy. Set
  `LDGR_ADMIN_EMAIL` or `LDGR_REGISTRATION=open` as described in
  [Quick start](#quick-start).
- **Admin panel rejects your login:** only accounts with the `admin` role can
  reach the panel. Make sure you're signing in as the bootstrap admin (the first
  account, or the `LDGR_ADMIN_EMAIL` account) — see
  [admin onboarding](#first-run-admin-onboarding).
- **Invitee can't redeem an invite token:** the client sign-up screens don't yet
  expose an invite-token field (see [Adding users](#adding-users)). Use
  `LDGR_REGISTRATION=open` to add people today.
- **Lost the Account Secret Key:** you can keep using devices already signed in,
  but can't add new ones. Your local vault is unaffected — it still opens offline
  with the master password. Recover the account via an existing device or admin
  assistance (see [the two-secret account model](#the-two-secret-account-model)).

## See also

- [Cross-Client Sync Setup guide](sync-setup.md) — the deep, click-by-click client
  walkthrough (register devices, sync a transaction, resolve conflicts).
- [ADR-008 — Self-Hosting & Two-Secret Account Auth](adr/008-self-hosting-and-account-auth.md)
  — the design rationale behind the account model, Emergency Kit, and admin UI
  placement.
- [ADR-003 — Sync & Conflict Resolution](adr/003-sync-conflict-resolution.md)
