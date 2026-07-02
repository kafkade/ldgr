# Cross-Client Sync Setup (Self-Hosted ldgr-server)

This guide walks an end user through standing up their own **ldgr-server**, creating
an account, registering more than one device, syncing a transaction between two
clients, reviewing conflicts, and understanding what the server can and cannot see.
You do **not** need to read any source code to follow it.

ldgr-server is an **encrypted blob relay**. It stores and serves encrypted blobs but
never decrypts them — it never sees your password or your plaintext financial data.
All encryption happens on your devices.

> **Running the server for others?** This guide is the end-user client
> walkthrough. For the operator's side — first-run admin onboarding, registration
> policy, adding users, and the two-secret account model — see the
> [Self-Hosting guide](self-hosting.md).

> **Licensing note:** the sync server (`crates/ldgr-server/`) is licensed under
> AGPL-3.0. The rest of ldgr is Apache-2.0.

## What you'll achieve

```text
  Device A  ──push──▶  ┌──────────────┐  ◀──pull──  Device B
 (your vault)          │  ldgr-server │            (your vault)
   add a txn  ────────▶│  (encrypted  │───────────▶  txn appears
                       │   blobs only)│
                       └──────────────┘
```

Both devices use the **same account** and the **same vault ID**; the server only
relays encrypted batches between them.

---

## Step 1 — Deploy ldgr-server

### Option A — Docker (recommended)

```sh
# Build the image from the repo root
docker build -t ldgr-server -f crates/ldgr-server/Dockerfile .

# Run it, persisting the database to a named volume
docker run -p 8080:8080 -v ldgr-data:/data ldgr-server
```

The image is preconfigured for container use: it binds `0.0.0.0:8080` and writes its
database to `/data/ldgr-server.db` (the `/data` volume), running as a non-root user.
A named volume (`ldgr-data`) keeps your accounts and blobs across restarts.

> **Heads-up — you probably want open registration for personal use.** See
> [Registration policy](#registration-policy-read-this-before-you-register) below.
> The common first run is:
>
> ```sh
> docker run -p 8080:8080 -v ldgr-data:/data -e LDGR_REGISTRATION=open ldgr-server
> ```

### Option B — Run directly

```sh
cargo run -p ldgr-server
```

Run directly, the server uses the binary defaults: it binds **`127.0.0.1:8080`
(loopback only)** and writes `ldgr-server.db` in the current directory. Loopback is
fine for testing on the same machine, but **other devices cannot reach it**. To serve
other devices, either bind all interfaces:

```sh
LDGR_BIND_ADDR=0.0.0.0:8080 cargo run -p ldgr-server
```

or (recommended) keep it on loopback and put a TLS-terminating reverse proxy in front
of it — see [Networking & TLS](#networking--tls).

### Configuration (environment variables)

All settings are read from the environment at startup. This table is the complete,
authoritative list.

| Variable | Default | Description |
| --- | --- | --- |
| `LDGR_BIND_ADDR` | `127.0.0.1:8080` | Listen address. The Docker image overrides this to `0.0.0.0:8080`. |
| `LDGR_DB_PATH` | `ldgr-server.db` | SQLite database path. The Docker image overrides this to `/data/ldgr-server.db`. |
| `LDGR_REGISTRATION` | `invite-only` | Who may register: `open`, `invite-only`, or `admin-only`. Any unknown value falls back to `invite-only`. |
| `LDGR_ADMIN_EMAIL` | (unset) | If set, the account that registers with this identity becomes the admin on first boot and bypasses the registration policy. |
| `LDGR_SESSION_TTL_HOURS` | `720` | Session (login token) lifetime, in hours. Default is 30 days. |
| `LDGR_RELAY_TTL_MINUTES` | `10` | Key-exchange relay offer lifetime, in minutes. |
| `LDGR_MAX_BLOB_BYTES` | `52428800` | Maximum encrypted blob size, in bytes. Default is 50 MB. |
| `LDGR_SRP_HANDSHAKE_TTL_SECS` | `120` | How long an in-progress login handshake stays valid, in seconds. |
| `LDGR_DEFAULT_QUOTA_BYTES` | `1073741824` | Default per-user storage quota, in bytes. Default is 1 GiB. |
| `LDGR_SERVER_NAME` | `ldgr-server` | Cosmetic server name advertised by the discovery endpoint. Never used for auth. |

> **Note:** the registration policy variable is `LDGR_REGISTRATION` (not
> `LDGR_REGISTRATION_POLICY`).

### Registration policy (read this before you register)

This is the single most common thing that blocks a new self-hoster, so read it
carefully.

The default policy is **`invite-only`**. The three policies are:

| Policy | Behaviour |
| --- | --- |
| `open` | Anyone can self-register. Best for a personal or family instance. |
| `invite-only` (default) | New accounts require an admin-issued invite token. |
| `admin-only` | Public self-registration is refused entirely. |

**The crucial detail: the very first account always succeeds.** On a fresh server
(empty user table), the first account to register becomes the **admin** and bypasses
the policy — even under the default `invite-only`. (If you set `LDGR_ADMIN_EMAIL`,
the account registering with that identity becomes the bootstrap admin instead.)

What this means in practice:

- **Syncing one account across your own devices works out of the box.** Your first
  device *registers* the account (and becomes admin); every other device just *logs
  in* with the same credentials — no new registration, so the policy never blocks you.
- **Adding a second, distinct account** (for example, a partner with their own login)
  is what `invite-only` blocks. Under the default policy that second registration
  fails with `registration is invite-only` until an admin issues an invite token.

For a personal or household instance, the simplest path is to set
`LDGR_REGISTRATION=open` so everyone in the household can self-register.

> **Issuing invite tokens** is currently an admin-API-only operation
> (`POST /api/v1/admin/invites`) and is **not yet surfaced in any client app or the
> CLI**. If you need `invite-only` today, you must call that authenticated admin
> endpoint yourself. For most self-hosters, `LDGR_REGISTRATION=open` is the
> straightforward choice.

### Networking & TLS

The server speaks **plain HTTP** — it does not terminate TLS itself. For any real
multi-device deployment over a network, run it behind a reverse proxy that terminates
HTTPS (Caddy, nginx, or Traefik) and forwards to the server.

Example with [Caddy](https://caddyserver.com/) (automatic HTTPS):

```caddyfile
sync.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

Then your clients use `https://sync.example.com` as the server URL. Keep the server
itself bound to loopback (`127.0.0.1:8080`, the default) so it is only reachable
through the proxy.

### Verify it's running

The server exposes unauthenticated liveness endpoints:

```sh
curl http://localhost:8080/api/v1/server/ping     # tiny liveness probe
curl http://localhost:8080/health                 # health check
curl http://localhost:8080/api/v1/server/info     # discovery document
```

A successful response confirms the server is up and the URL is correct.

---

## Step 2 — Create an account and register each device

How you authenticate depends on what your server advertises at
`GET /api/v1/server/info` (every client checks this when you enter the server URL):

- **Two-secret servers (recommended, ADR-008).** You sign up with a **password**
  **and** a generated **Account Secret Key**. At sign-up the client shows an
  **Emergency Kit** — your address, account hint, Secret Key, and a QR code — **once**.
  Save it: the master password alone opens your vault offline, but adding a **new
  device** needs the Secret Key too (typed or scanned from the Kit). This means an
  attacker who steals the server database still can't brute-force a weak password.
- **Single-secret servers (legacy).** Password-only SRP-6a, exactly as before. Clients
  fall back to this automatically when the server doesn't advertise two-secret auth.

Either way your password never leaves the device (SRP-6a): the server stores only a
verifier, never the password itself (see [Threat-model recap](#threat-model-recap)).

Pick a **vault ID** and use the **same vault ID and the same account** on every device
you want to keep in sync.

> **Save your Emergency Kit.** The Secret Key is shown **once**, at sign-up. If you
> lose it you can still use every device already signed in, but you won't be able to
> add new ones. Store it in a password manager or print it.

### Web app

In the vault's **Settings → Sync (ldgr-server)** panel:

1. Enter the **Server URL** (e.g. `https://sync.example.com`) and click **Connect**.
   The panel shows the server name, protocol version, and whether it uses two-secret
   auth.
2. Fill in **Vault ID**, **Username**, and **Password**.
3. On your first device, click **Create Account**. On a two-secret server the app
   generates your Secret Key and shows the **Emergency Kit** — save it (copy /
   download / print) before continuing.
4. On later devices, click **Sign In**. On a new device the app prompts for your
   **Secret Key** (paste it from the Kit); after that it's remembered for that device.

The panel shows **🟢 Authenticated** with a short device ID when you're signed in.

### iOS / iPadOS app

In **Sync** settings:

1. Enter the **Server URL** and tap **Connect** to validate it and read the server's
   capabilities.
2. Enter **Username**, **Password**, and **Vault ID**.
3. Tap **Create Account** on your first device — the app generates your Secret Key and
   presents the **Emergency Kit** (share sheet / screenshot / QR) to save once. On a
   new device tap **Sign In on This Device** and enter the **Secret Key** from your
   Kit; on a device that already has it, just tap **Sign In**.

The app stores your session token **and** Account Secret Key in the Keychain
(device-only) and registers the device. **Sign Out** clears the session token but
keeps the Secret Key, so you can sign back in with just your password.

### CLI

Run the interactive setup from your vault:

```sh
ldgr sync setup
```

Enter the **Server URL**; the CLI validates it against `/server/info` and prints the
server name, protocol version, and auth mode. Then:

- **Two-secret server, first device:** the CLI generates your Account Secret Key,
  registers, and renders your **Emergency Kit** — boxed text **and** a scannable
  terminal QR code, with an option to export it to a `0600` file. The Secret Key is
  stored in `sync-credentials.json` (`0600`); your master password is never stored.
- **Two-secret server, new device:** paste (or point the CLI at a saved Kit file
  containing) your **Secret Key**; the CLI derives the login and signs in.
- **Single-secret server:** enter **Username** and **Password**; if the account
  doesn't exist the CLI offers to register it.

On success it saves a non-secret `sync-config.json` and the SRP session token in
`sync-credentials.json` (permissions `0600`). After setup:

```sh
ldgr sync push      # upload your local encrypted batches
ldgr sync pull      # download other devices' encrypted batches
ldgr sync status    # show provider, device ID, last sync, pending counts
```

> **Important CLI limitation (today):** `ldgr sync pull` downloads other devices'
> batches into a local inbox but does **not** yet apply them to your vault, and the
> CLI has no conflict-review command. So the CLI can *publish* and *fetch* changes,
> but completing a cross-device merge currently requires the **Web** or **iOS/macOS**
> app. Use those for the end-to-end walkthrough below.

---

## Step 3 — Sync a transaction between two devices

This walkthrough uses two clients signed in to the **same account** with the **same
vault ID** (two browsers, two devices, or one of each). The Web and iOS/macOS apps
both apply pulled changes; the CLI does not yet (see the note above).

1. **Device A** — add a transaction in the app as you normally would.
2. **Device A** — open Sync and click **Sync now** (Web) or **Sync Now** (iOS/macOS).
   This encrypts your pending changes into a batch and uploads it. The Web app reports
   the outcome, e.g. *"Synced: pushed 1, applied 0, conflicts 0, skipped 0."*
3. **Device B** — open Sync and click **Sync now**. Device B downloads Device A's
   batch and applies it. The Web app reports *"applied 1"*, and the new transaction
   appears in Device B's vault.

That's a complete round trip: the transaction moved from A to B, end to end, with the
server only ever holding encrypted blobs.

> **How convergence works:** devices converge by exchanging and replaying encrypted
> **event batches** — each device pushes its pending events and pulls + applies the
> others'. There is no QR-code or snapshot "fast onboarding" path wired into the
> clients yet; a brand-new device simply signs in to the same account/vault and syncs
> to pull the existing batches.

---

## Step 4 — Review and resolve conflicts

ldgr never silently drops one of two concurrent edits. Following
[ADR-003](adr/003-sync-conflict-resolution.md), transactions are **atomic**, edits to
**different** entities merge automatically, but edits to the **same** entity on two
devices are flagged for **your review** — there is no silent last-write-wins. After
each sync, double-entry invariants are re-validated.

A conflict typically arises when you edit the same transaction on two devices before
they sync.

### Web app

When conflicts exist, the Sync panel shows a **"Conflicts to review"** section listing
each conflicting entity with **This device** and **Remote** summaries side by side.
For each one, choose **Keep mine** or **Keep remote**.

### iOS / iPadOS / macOS app

The Sync screen shows a **Conflicts** section with a count; tapping it opens a
conflict list where you review each entry and pick which version to keep. The status
view also surfaces an **Unresolved Conflicts** count.

### CLI

`ldgr sync status` reports pending push/pull counts, but the CLI does not provide a
conflict-resolution command yet. Resolve conflicts in the Web or iOS/macOS app.

---

## Threat-model recap

For the full rationale see
[ADR-008](adr/008-self-hosting-and-account-auth.md). In short:

### What the server can see

| The server holds | Sensitive? |
| --- | --- |
| Encrypted vault blobs | No — AES-256-GCM ciphertext, size-bucket padded |
| Your `(salt, verifier)` | No — the verifier is `g^x mod N`; it is not your password |
| Your email / username | Identity only |

**What the server never sees:** your password, your encryption keys, or any plaintext
financial data. With SRP-6a, your password is never transmitted during sign-in — the
server only ever checks a zero-knowledge proof against the stored verifier.

**Two-secret authentication (2SKD).** When your server advertises it, sign-in uses
**two** secrets: your **password** and a generated **Account Secret Key**, combined via
**Two-Secret Key Derivation** (ADR-008) — a 1Password-style key mixed into the SRP
exponent, plus a printable **Emergency Kit**. Even an attacker who steals the entire
server database cannot brute-force a weak password offline, because the Secret Key
(never sent to the server) is required to reconstruct the SRP verifier. The Secret Key
is **auth/sync-only**: your vault still opens offline with the master password alone.
The account id needed to derive the key is generated by the client at sign-up, stored
by the server, and returned at `login/init`, so new-device sign-in stays "email +
password + Secret Key". Servers that don't advertise two-secret auth fall back to
single-secret (password-only) SRP-6a automatically.

---

## Troubleshooting

| Symptom | Likely cause / fix |
| --- | --- |
| `registration is invite-only` (403) on register | Default policy. Set `LDGR_REGISTRATION=open`, or register your first/admin account first, or issue an invite token via the admin API. |
| Other devices can't reach the server | Running directly binds loopback (`127.0.0.1`). Set `LDGR_BIND_ADDR=0.0.0.0:8080` or front it with a reverse proxy. |
| TLS / certificate errors | The server is plain HTTP. Terminate HTTPS at a reverse proxy (Caddy/nginx/Traefik) and point clients at the `https://` proxy URL. |
| A transaction won't appear on the other device | Confirm both devices use the **same account** and the **same vault ID**, and that you ran **Sync now** on both. Remember the CLI does not apply pulled batches yet — use the Web or iOS/macOS app. |
| Data lost after restarting the container | Persist the database with a volume: `-v ldgr-data:/data`. |
| Login token stopped working after ~30 days | Sessions expire per `LDGR_SESSION_TTL_HOURS` (default 720 h). Sign in again. |

---

## See also

- [ADR-008 — Self-Hosting & Two-Secret Account Auth](adr/008-self-hosting-and-account-auth.md)
- [ADR-003 — Sync & Conflict Resolution](adr/003-sync-conflict-resolution.md)
- [Sync Batch-Blob Format](sync-blob-format.md)
