# Metadata and privacy inventory

This document inventories what ldgr stores or exposes **outside encrypted financial
payloads**. It describes the current implementation, not a promise of anonymity or a
future data-minimization design. It complements the
[Vault format threat model](./threat-model.md), which covers cryptographic protections
and the local-at-rest boundary.

Scope: the self-hosted sync server, its operational logging and transport, and the
local storage exceptions relevant to that boundary. Third-party blob transports and
hosting providers can collect additional metadata under their own policies.

## 1. Where zero knowledge ends

The client encrypts vault items and canonical sync batches with AES-256-GCM.
Native working stores use SQLCipher; the web seals its in-memory ledger database
before persisting it. The sync server is not given the vault decryption key and
cannot decrypt those financial payloads merely by reading its database.

This does **not** mean the server sees only ciphertext:

- Account identities, authentication records, invite relationships and session
  timestamps are readable.
- Vault/device identifiers and relationships, blob paths, sizes, hashes and upload
  timestamps are readable.
- Current CLI device descriptors expose machine name, platform and last-sync time,
  despite the server field being named `encrypted_info`. The CLI currently supplies
  an empty vector clock; it is not exposing current per-device event counts there.
- Pairing relay bodies include public keys and a public-key/return-offer message;
  the vault-key delivery is encrypted. Not every relay payload is ciphertext.
- Authentication and admin logs contain personal data.

These protections assume an honest, uncompromised client and protection of its
keys. They do not protect secrets from modified client code or a hostile OS.
Authentication credentials and session tokens are distinct from vault decryption
keys; TLS is required to protect the transport's non-vault-encrypted information.

## 2. Server database inventory

The server opens an ordinary SQLite database. It does not apply the client's
SQLCipher working-store key to this database. Host/disk encryption, if configured
by the operator, is a separate control.

All fields below are readable from a server database copy, except where an
application-encrypted payload is explicitly identified. Binary storage (`BLOB`)
and hex encoding do not themselves imply encryption.

An honest-but-curious operator and someone holding a database copy can read the
same stored values and join their relationships. The operator additionally observes
live requests and may control logs and backups. A stolen database **alone** does
not contain the client's vault key, raw session/invite tokens or Account Secret Key;
it does contain sensitive authentication verifiers and all retained metadata.

| Table | Stored fields | What the operator or database thief learns |
|-------|---------------|-------------------------------------------|
| `users` | `id`, `username`, `email`, `account_id` | Stable account identifiers and sign-in identity; links to vaults, sessions, invites and relay activity. Username normally represents an email, though the API accepts other strings |
| `users` | `salt`, `verifier`, `auth_scheme`, `secret_key_version` | SRP authentication material and the account's authentication scheme/version. A verifier is not a plaintext password, but is an authentication-sensitive record |
| `users` | `account_kdf_salt`, `account_kdf_mem_kib`, `account_kdf_iters`, `account_kdf_parallelism` | Account-scoped Argon2 salt and cost parameters, including whether older/nullable KDF material is present |
| `users` | `role`, `status`, `storage_quota_bytes`, `invited_by`, `created_at`, `updated_at` | Privilege/status, quota policy, inviter relationship and account creation/update activity |
| `invites` | `token_hash`, `email`, `role`, `created_by`, `created_at`, `expires_at`, `redeemed_at`, `redeemed_by` | Intended recipient/privilege, issuing account, issuance/expiry/redemption timing and the account that redeemed it. Token hash is a linkable invite id, not the raw redeemable token |
| `sessions` | `token_hash`, `user_id`, `created_at`, `expires_at` | Login/session issuance and validity windows, linked to an account. Hashes are not raw bearer tokens. There is no device id, IP address or last-use timestamp in this table |
| `vaults` | `id`, `user_id`, `created_at` | Vault count and ownership per account, plus server-side vault registration time. New server-minted ids are random; retained legacy/client-requested ids can still have recognizable meaning |
| `blobs` | `path`, `vault_id`, `data`, `size`, `content_hash`, `created_at` | Path relationships identify vault, blob kind and batch device/id or snapshot id. Canonical batch `data` is encrypted. Exact uploaded byte length, a hash of those bytes and server insertion time remain readable |
| `devices` | `id`, `vault_id`, `encrypted_info`, `updated_at` | Registered device-descriptor count and vault association, descriptor update timing and payload byte length. Current CLI descriptor contents are readable as described in §1; the column name is not an encryption guarantee |
| `relay_offers` | `id`, `user_id`, `offer_data`, `response_data`, `created_at`, `expires_at` | Pairing/relay activity per account, offer relationships, timing and payload lengths; public keys/return-offer information in the current pairing exchange. Vault-key delivery remains encrypted |
| `settings` | `key`, `value` | Persisted registration-policy, default-quota and maximum-blob-size overrides; server policy/capacity choices, not an encrypted settings store |

### 2.1 Authentication material is separate from vault material

The `users.salt` is the SRP salt; `account_kdf_salt` and the three account-KDF
parameters describe a separate account-scoped Argon2 derivation. They are not the
vault header's salt/parameters and are not secret key material.

Two-secret clients derive their account auth key from the password and account KDF,
then mix the client-held Account Secret Key and account id into the SRP input.
Password-only/legacy accounts may lack account-KDF fields. A database copy does not
contain the client-held Secret Key.
Authentication-verifier exposure must nevertheless be evaluated by scheme and by
what other client secrets an attacker has; it is not equivalent to vault decryption
and should not be dismissed as harmless metadata.

The account KDF is stored server-side, returned during sign-in initialization and
included in the account Emergency Kit. The Emergency Kit's Account Secret Key is
not the vault recovery key. Protect user-retained kits and client credential stores
separately from server metadata.

### 2.2 Activity and relationship inferences

Joining account, vault, blob and device rows associates a sign-in identity with its
vaults and upload sources. Invite rows reveal an invitation graph and lifecycle.
Session timestamps reveal successful login/session activity, **not** every request
or a reliable count of physical devices: one device can create several sessions.
Device rows count registered descriptors, not necessarily distinct physical devices.

Blob insertion times form an **upload timeline**, not the financial transactions'
dates. The core batch exporter also uses time-bearing UUIDv7 batch ids, so those
paths disclose client-side identifier creation time as well. Blob counts and size
ranges provide a rough volume/activity signal. Canonical batches can contain multiple
events and different entity kinds, so neither count nor byte length proves an exact
number of transactions or postings. Polling and unchanged-data downloads are not
recorded in these rows; a live operator may still observe them.

### 2.3 Padding and hashes: what is actually hidden

The canonical batch path serializes an event batch and encrypts it through the
core `encrypt_item` envelope. That function adds a four-byte plaintext-length prefix
and pads to **512 B / 2 KiB / 8 KiB / 32 KiB**, then 32 KiB multiples for larger
payloads, before AES-256-GCM encryption. Exact plaintext length is hidden within a
bucket; large batches still disclose a 32 KiB-range size signal.

The uploaded envelope is JSON, including a ciphertext byte array and cryptographic
framing. Consequently, the server's `size` is the **whole uploaded body's byte
length**, not a literal padding-bucket value. The ciphertext array length still
reveals the bucket, with the authentication-tag overhead. Variable JSON byte
representation does not make the bucket secret.

The server computes `content_hash` as SHA-256 of the uploaded body. This can link
byte-identical uploads or copies; it is not a hash of plaintext transactions.
Fresh random encryption nonces/item keys mean separately sealed copies of the same
plaintext need not have the same hash.

Snapshot upload is supported by the API, transports and FFI as supplied bytes;
those interfaces are not themselves a snapshot encryption/padding implementation.
The current built-in snapshot type/planning helpers must not be confused with a
production snapshot-creation pipeline. Do not extend the confirmed batch-padding
guarantee to arbitrary snapshot uploads or device/relay bodies.

## 3. Transport metadata is a separate exposure

The stored schema does not include source IP addresses, user agents or a full
request/access history. Their absence from SQLite does not make them unobservable:

- A server, reverse proxy or hosting provider can observe connection IP addresses,
  request timing/frequency and transferred sizes.
- TLS terminators and the server can observe request paths/identifiers, headers and
  request patterns, including login, upload, listing and pairing activity.
- A passive network observer can still see connection endpoints, timing and traffic
  volumes under TLS, but not the encrypted HTTP request contents.
- Proxies, CDNs, service managers and container logging drivers may retain
  additional records independently of the app's database or log filter.

Use HTTPS and restrict exposure of backend HTTP listeners. TLS protects transport
credentials and request metadata; vault encryption alone protects neither identity
nor all device/operational information.

## 4. Logging footprint

The server uses `tracing_subscriber::fmt()` with stdout as its default writer.
The default filter enables INFO for server and HTTP middleware; operators can
change the filter. Stdout is normally collected by the container logging driver
or service manager, whose destination and retention are operator-controlled.

| Log category | Level | Observable fields |
|--------------|-------|-------------------|
| Registration, bootstrap-admin registration, successful login | INFO | Username/sign-in identity; registration role |
| Admin user creation/deletion and status/role/quota changes | INFO | Acting admin id, target user id, username on creation/deletion, changed status/role/quota |
| Invite issuance/revocation | INFO | Acting admin id; issuance additionally records invite hash/id, role and optional recipient email |
| Registration-policy/default-quota/max-blob-size changes | INFO | Acting admin id and new policy/value |
| Startup and CORS configuration | INFO | Bind address and allowed-origin count when CORS is enabled |
| Invalid origin or malformed persisted setting | WARN | Rejected origin or setting key/value |
| Database/task failures | ERROR | Error strings; diagnostics should not be assumed metadata-free |

**Logs contain personal data.** Username normally means email; account ids,
recipient email and administrative relationships remain identifying even when
financial payloads are encrypted.

The inspected explicit auth/admin audit fields do not print passwords, SRP salts or
verifiers, raw bearer tokens or vault keys. This is a limited observation about the
current fields, **not a blanket no-secret-logging guarantee**. Richer HTTP tracing
and separately configured access logs can capture sensitive or credential-bearing
request metadata. Successful credential revocation changes its usability; failed
handling can leave a logged credential valid. Do not assume all credentials
appearing in such logs are either live or safely invalidated.

The shipped Caddy configurations do not enable an access-log directive. Operators
may enable access logs or add other proxies, and must review those independently.
Neither the server nor the shipped Compose bundle defines log rotation or a
retention duration.

## 5. Retention: expiration is not erasure

| Record/copy | Current lifecycle |
|-------------|-------------------|
| Users and vault-associated data | No age-based retention policy. Account deletion removes the user and cascades to sessions, vaults, blobs, devices and relay offers through foreign keys |
| Sessions | Default validity is 720 hours (30 days). Expired rows fail validation; there is no active scheduled session-row purge. Application sign-out must not be assumed to erase or immediately invalidate the server record |
| Invites | Expiry is optional. Redemption records who/when and retains the row. Revocation removes an unredeemed row; expired/redeemed rows have no scheduled purge, and account deletion does not cascade to invite records |
| Relay offers | Default validity is 10 minutes. Expired offers cannot be retrieved through normal handlers; expired rows are cleaned up when another offer is created, not at a scheduled deadline |
| Blobs and devices | No age-based server purge. Device removal deletes its descriptor, not historical batch blobs; snapshot retention/planning helpers are not a scheduled server deletion policy |
| Settings | Persisted overrides remain until changed; no automatic expiry |
| Logs and operator backups | No application-defined retention/rotation policy; deleting a database record does not delete earlier logs or backup copies |

SQL deletion is a logical lifecycle operation, not a promise of forensic erasure
from database free pages, WAL files, filesystem snapshots or backups. Encryption of
financial payloads does not erase plaintext metadata in those historical copies.

## 6. Local storage boundaries

See threat-model [§4.3](./threat-model.md#43-device-thief-with-a-locked-device-or-disk-image--in-scope-at-rest)
for the per-platform encrypted working-store model and
[§4.4](./threat-model.md#44-forensic-examiner-with-a-memory-dump-or-swap-file--partial--best-effort)
for memory/swap limitations.

The ledger store's encryption does not cover every local record:

- CLI and Apple working-store migrations retain `vault.db.plaintext.bak` for
  backout; they do not automatically remove that readable copy or older backups.
- CLI session metadata exposes the vault path and timestamps. The session key is
  cached in the OS keystore; Apple biometric/user-presence unlock caches the vault
  session key in the device-only Keychain, not the MEK.
- CLI sync bearer tokens and the Account Secret Key are stored separately as
  readable JSON protected by owner-only file permissions on Unix, not by the
  encrypted working store. On Apple clients those server credentials use the
  device-only Keychain after first unlock, separately from the biometric-gated
  vault session key.
- Apple sync configuration retains server URL, username, vault id and pull cursor
  in `UserDefaults`. Native widget summaries and pending expense-intent data are
  also serialized outside the keyed database. The widget manager has an explicit
  clear-on-lock path; this is not a forensic-erasure guarantee.
- Web ledger bytes are sealed before IndexedDB persistence, but vault names are
  plaintext IndexedDB keys. The admin session's username, server URL and bearer
  token are kept separately in `sessionStorage`, not encrypted with the ledger.
  Session-scoped browser storage is not an application-level encrypted store or
  a general disk-erasure guarantee.
- watchOS has no vault DB, but shared `UserDefaults` contains JSON financial
  summaries for the app/widgets. Those derived values rely on platform protection,
  not the vault envelope.
- Native SQLCipher memory locking is not enabled; key material, decrypted pages
  and web/WASM plaintext buffers remain process-memory/swap concerns.

## 7. Operator responsibilities

- Restrict database, volume, backup and log access; treat identity relationships,
  auth verifiers, invite history and activity timestamps as sensitive data.
- Decide and document retention separately for database rows, logs, proxies,
  container logging, snapshots and backups. Verify actual cleanup, not just TTLs.
- Minimize logged identifiers and request metadata. Avoid collection of
  credential-bearing metadata and do not add credential/body/header logging.
  Redaction and collection controls are operator responsibilities, not existing
  guarantees of the vault format.
- Configure bounded log rotation and access controls in the hosting/logging stack;
  include any exported logs or observability service in the privacy assessment.
- Plan migration-backup disposal after verifying recovery/backout needs. Include
  historical backups and recognize that ordinary file deletion is not secure erasure.
- Use TLS, keep the backend private and account for metadata visible at the TLS
  terminator. An encrypted financial payload does not make its transport anonymous.

## 8. References and maintenance

- [Vault format threat model](./threat-model.md)
- [ADR-008: Self-hosting and account authentication](../adr/008-self-hosting-and-account-auth.md)
- [ADR-010: Per-platform local-at-rest model](../adr/010-local-working-store-at-rest-encryption.md)
- [ADR-011: Tenant-scoped vault identifiers](../adr/011-tenant-scoped-vault-identifiers.md)
- [Self-hosting guide](../self-hosting.md)
- [Server database schema](../../crates/ldgr-server/src/storage/schema.rs)
- [Canonical batch framing](../../crates/ldgr-core/src/sync/framing.rs)
- [Envelope encryption and padding](../../crates/ldgr-core/src/crypto/envelope.rs)

Recheck this inventory when schema fields, authentication, client persistence,
device/relay payloads, log fields or cleanup invocation paths change. An encrypted
column name, a validity timestamp or a planning helper is not evidence of an
implemented confidentiality or retention guarantee.
