# Vault format threat model

> **Who this is for:** security-conscious users, auditors, and engineers who want a
> precise account of what ldgr's vault encryption protects, what it does not, and why.
>
> For a plain-English overview with no technical background required, see
> [How is my data protected?](./vault-overview.md). For the full system design, see the
> [ldgr Architecture](../ldgr-architecture.md) (§4 Encryption Architecture, §10 Sync
> Server). This document is the authoritative, expanded version of the threat-model
> summary in architecture §4.5. For the server's stored metadata, observable activity,
> logging and retention, see [Metadata and privacy inventory](./metadata-and-privacy.md).

ldgr is a **zero-knowledge, local-first** personal finance app. Financial data is
client-encrypted in the vault container and canonical sync batches; native working
databases are also encrypted at rest. This does **not** mean all local files, derived
financial caches, server records or logs are encrypted. The server can read account
identities and sync metadata, and some device information is currently observable.
This document describes the vault's cryptographic guarantees and the boundaries of
the local-at-rest model, not anonymity or protection of every copy of the data.

Vault cryptography is implemented under
[`crates/ldgr-core/src/crypto/`](../../crates/ldgr-core/src/crypto/); working-store and
session handling also depend on platform code. Cryptographic primitives come from
[RustCrypto](https://github.com/RustCrypto) crates.

---

## 1. Scope

**In scope:** the confidentiality, integrity, and authenticity of vault data at rest (on
disk), in transit (during sync), and — on a best-effort basis — in memory while the vault
is unlocked. The local working-store boundary is described in §4.3 and
[ADR-010](../adr/010-local-working-store-at-rest-encryption.md). Server metadata,
authentication records and operational logs are inventoried separately in
[Metadata and privacy inventory](./metadata-and-privacy.md).

**Out of scope:** the security of the device's operating system, the integrity of the app
binary itself, and any attack that observes the user directly (their screen, keyboard, or
person). These are explicitly enumerated in [§6](#6-what-the-vault-does-not-protect-against)
rather than silently assumed away.

---

## 2. Asset inventory

| Asset | Where it lives | How it is protected |
|-------|----------------|---------------------|
| **Financial transaction data** (accounts, postings, balances, budgets) | Vault items and canonical sync batches; platform working database | AES-256-GCM envelopes; SQLCipher on native working stores or a sealed container for web persistence |
| **Master password** | User input and temporary client memory | Not persisted as a credential or sent to the sync server; Argon2id derives key material (`kdf.rs`) |
| **Master Key (MK)** | Temporary memory during password derivation | Derived on unlock, never persisted; `Zeroize`/`ZeroizeOnDrop` (`keys.rs`) |
| **Master Encryption Key (MEK)** | Temporary memory during derivation and key wrapping | HKDF-derived from MK; zeroized on drop |
| **Vault Key (VK)** | Stored **wrapped** in the header; unwrapped into memory; optionally cached as the session key in the OS keystore/Keychain | AES-256-GCM-wrapped by MEK *and* by the recovery key; cached-key protection depends on the platform |
| **Database Key** | Memory during native working-store use | HKDF-SHA256 subkey of VK with info `ldgr-sqlcipher-key-v1`; used as the SQLCipher raw key; key type zeroized on drop |
| **Per-item keys (IK)** | Stored **wrapped** alongside each item | AES-256-GCM-wrapped by VK; one random key per item |
| **Vault recovery key** | Client memory during creation/recovery and the user's retained copy; distinct from the account-authentication Emergency Kit | 256-bit random; wraps VK independently of the password |
| **Vault metadata** (vault name, `created_at`) | Encrypted blob in the header (`encrypted_metadata`) | AES-256-GCM under VK — **not** readable without unlocking |
| **Header KDF parameters** (`salt`, `argon2_params`, `format_version`, `kdf_version`) | **Plaintext** in the header (`VaultHeader`) | Parseable without the password; only *indirectly* authenticated (see [§9](#9-future-improvements)) |
| **Legacy working-store backup** | `vault.db.plaintext.bak` after migration | **Not encrypted by migration**; retained for backout |
| **Local labels and derived caches** | Filesystem names/session metadata, web vault-name keys, native widget/intent data and watch summaries | Outside the vault-envelope guarantee; see §4.3 and the privacy inventory |

A useful consequence: an attacker holding the raw vault file can read the Argon2id
parameters and salt (needed to attempt unlock) but **cannot** read the vault's name,
creation time, or financial item payloads from that file alone. Names and activity may
still be exposed by its filename, filesystem timestamps or separate local records.

---

## 3. Security properties

The vault provides three properties, each backed by a specific mechanism:

- **Confidentiality.** Every item payload is encrypted with AES-256-GCM under a unique,
  random per-item key (`encrypt_item` in `envelope.rs`). Item keys are wrapped by the
  Vault Key; the Vault Key is wrapped by the password-derived MEK. Confidentiality
  requires those decryption keys to remain unavailable to an attacker; access to an
  unlocked/cached VK bypasses password wrapping.

- **Integrity.** AES-256-GCM is an AEAD: every ciphertext carries a 128-bit authentication
  tag. Any modification to a wrapped key or item ciphertext causes decryption to fail
  rather than return corrupted plaintext. Each item, and each key-wrap, is independently
  authenticated.

- **Authenticity / domain separation.** Each cryptographic role uses a distinct AAD or
  HKDF info string, so a ciphertext produced for one role can never be successfully
  decrypted in another:

  | Operation | Tag |
  |-----------|-----|
  | Wrap Vault Key with MEK | `ldgr-vault-wrap-v1` |
  | Wrap Vault Key with recovery key | `ldgr-recovery-wrap-v1` |
  | Wrap item key with Vault Key | `ldgr-item-wrap-v1` |
  | Seal item payload | `ldgr-item-seal-v1` |
  | Derive Auth Key (HKDF) | `ldgr-auth-v1` |
  | Derive MEK (HKDF) | `ldgr-enc-v1` |
  | Derive native working-store key from VK (HKDF) | `ldgr-sqlcipher-key-v1` |

- **Length hiding (partial).** Item payloads are padded to size buckets
  (512 B / 2 KB / 8 KB / 32 KB, then 32 KB multiples) before encryption (`envelope.rs`),
  hiding the exact plaintext length but revealing its bucket. Canonical sync batches
  use this envelope too; they contain multiple events, not necessarily one transaction.
  Padding does **not** hide envelope/blob counts or timing, and is not a promise about
  every body accepted by a sync transport.

---

## 4. Threat actors

Each actor below is given an explicit **in-scope** (the design defends against it) or
**out-of-scope** (acknowledged, not defended) classification.

### 4.1 Curious server / sync operator — **in scope**

*Capabilities:* full read/write access to every stored blob, the header, sync metadata,
and SRP-6a authentication records. This includes ldgr's own hosted server and any
self-hosted or third-party blob store.

*Analysis:* the honest client encrypts financial payloads before uploading them; the
sync server does not receive the vault decryption key. **SRP-6a** authentication does
not send the password. The two-secret scheme uses separate account-scoped KDF
material and the client-held Account Secret Key; password-only/legacy records may
lack that account KDF. Authentication records are not server-held vault decryption keys.

The server is **not** a metadata-free store. Its ordinary SQLite database contains
account identities, authentication records, invites, sessions, account-to-vault
relationships, device identifiers, blob paths, sizes, hashes and insertion times.
Current CLI device descriptors expose machine name, platform and last-sync time.
Authentication/admin logs also contain personal data. Canonical batch padding limits
length disclosure, but does not hide blob counts or activity; counts are not exact
transaction counts. See the [privacy inventory](./metadata-and-privacy.md) for the
complete boundary, including relay payloads and retention.

### 4.2 Network attacker (MITM on the sync transport) — **in scope**

*Capabilities:* intercept, replay, or modify traffic between the client and sync server.

*Analysis:* financial payloads in canonical batches are AES-256-GCM-encrypted
**before** transport; a modified envelope fails authentication on decrypt.
TLS remains necessary for server authentication and protection of session credentials,
account/device information and request metadata that are not vault-encrypted.
End-to-end encryption is not a replacement for HTTPS.

Residual exposure includes traffic timing/size analysis and drop/delay attacks.
AEAD authentication alone does not establish freshness or prevent replay of an
unchanged valid ciphertext.

### 4.3 Device thief with a locked device or disk image — **in scope (at rest)**

*Capabilities:* physical possession of a powered-off/locked device, or a forensic copy of
its disk, including the raw vault file.

*Analysis:* protection covers the encrypted vault container and the platform's
encrypted working store, provided keys and plaintext copies are unavailable:

| Platform | Working-store persistence | Boundary |
|----------|---------------------------|----------|
| CLI | `vault.db` is SQLCipher-encrypted with an HKDF subkey of VK | Session key cached in the OS keystore; `session.json` holds plaintext vault path and timestamps, not the key |
| iOS/iPadOS/macOS | Shared FFI opens `vault.db` with the same SQLCipher key derivation | Biometric/user-presence unlock can cache the vault session key in the device-only Keychain; sync configuration and widget/intent caches are separate |
| Web | sql.js runs in memory; database exports are encrypted as a vault item before the sealed container is saved to IndexedDB | The vault name used as the IndexedDB key is plaintext; admin session storage is separate from the sealed ledger |
| watchOS | No vault database | Derived financial summaries are cached as JSON in shared `UserDefaults`, outside vault encryption |

The native raw-key derivation is
`HKDF-SHA256(VK, info = "ldgr-sqlcipher-key-v1")`; the database key is not an
independently stored password. See [ADR-010](../adr/010-local-working-store-at-rest-encryption.md).

An attacker with the container can parse its salt and KDF parameters and attempt
offline password guesses. Cost depends on the profile in [§7](#7-argon2id-parameter-rationale);
encryption does not rescue a weak password or an accessible cached key.
A locked OS screen does not itself prove that the application session or cached key
has been cleared.

**Migration does not eliminate every plaintext copy.** CLI and FFI migrations verify
the encrypted replacement but deliberately retain the original as
`vault.db.plaintext.bak`. It is not automatically removed and may also be present in
older backups or snapshots. Until those copies are dealt with, readable financial
data remains outside the encrypted working-store boundary. Removing a file is not
a secure-erasure guarantee.

Capture of an unlocked vault, accessible OS-keystore session key or decrypted memory
is outside the locked-file guarantee (see §4.4 and §6).

### 4.4 Forensic examiner with a memory dump or swap file — **partial / best-effort**

*Capabilities:* a RAM image or swap/hibernation file captured while the vault was unlocked,
or shortly after.

*Analysis:* derivation and decryption require plaintext key material in process
memory. MK/MEK may be temporary; VK, in-use item/database keys and decrypted working
pages can be live while data is being used. Core key types implement
`Zeroize`/`ZeroizeOnDrop`, and their `Debug` implementations redact key bytes.
These protect those values on drop or formatting; they do not redact a raw memory
dump or guarantee erasure of every exported key copy or plaintext buffer.

CLI session expiry is checked when a session is loaded; it is not a background
deadline that guarantees immediate keystore erasure while the app is idle.
Application locking and key eviction do not undo plaintext already captured.

Native working stores deliberately do **not** enable SQLCipher's
`cipher_memory_security` allocator-locking pragma, because of its Windows
working-set failure and platform costs. At-rest file encryption does not imply
locked in-memory pages. SQLCipher pages, Rust buffers and the web/WASM heap can
therefore remain exposed to process-memory capture or OS-managed swap/hibernation.
These are best-effort mitigations, not a memory-confidentiality guarantee.

### 4.5 Another tenant on a shared sync server — **in scope**

*Capabilities:* a fully authenticated account on the same self-hosted or multi-tenant
server, able to make any authenticated API call and to guess or enumerate identifiers.

*Analysis:* every vault-scoped endpoint (batches, snapshots, devices) checks that the
authenticated account owns the vault before doing anything, and answers `404 Not Found`
rather than `403 Forbidden`, so a tenant cannot even confirm whether another tenant's
vault exists. Relay offers used for device pairing are likewise bound to the account that
created them. Newly server-minted vault identifiers carry 128 bits of CSPRNG entropy;
existing identifiers are retained for compatibility and need not be random. A request for an
identifier another account already holds is answered with a freshly minted one instead of
a conflict, so a hostile tenant cannot deny service by squatting on it. Even a tenant who
learns another's identifier reads nothing, because the ownership check is independent of
how the identifier was obtained. **A co-tenant cannot read or write another tenant's
data through these handlers, or block a vault claim by identifier squatting.**
Shared-resource contention remains an availability risk; authorization does not promise
isolation of disk, bandwidth or CPU capacity. See
[ADR-011](../adr/011-tenant-scoped-vault-identifiers.md).

### 4.6 Malicious app co-resident on the same device — **out of scope**

*Capabilities:* another application running on the same device, possibly with elevated or
root privileges.

*Analysis:* if a hostile process can read ldgr's memory, hook its syscalls, or has root,
no application-level cryptography can defend against it — it can read keys directly while
the vault is unlocked, or capture the password as it is typed. ldgr relies on the OS
process/sandbox boundary here and does not claim to protect against a compromised device.
Out of scope; see [§6](#6-what-the-vault-does-not-protect-against).

---

## 5. What the vault protects against

| Surface | Threat | Mechanism |
|---------|--------|-----------|
| In transit / server | **Disclosure of encrypted financial payloads** | Client-side AES-256-GCM; sync server is not given the vault decryption key. Account records, device information and logs are not covered |
| In transit | **Network interception** | Client-side AES-256-GCM for financial payloads; TLS for transport credentials/metadata and server authentication |
| At rest | **Encrypted container / working-store exposure** | Argon2id-derived MEK wraps VK; encrypted vault items; keyed SQLCipher native working stores or sealed web database exports |
| At rest | **Offline brute force** | Argon2id memory-hard KDF makes each password guess costly; per-platform parameters tuned for resistance (§7) |
| Shared server | **Cross-tenant access & identifier squatting** | Account-scoped ownership checks answer `404` for unowned vaults; newly minted identifiers are random and a taken requested identifier is replaced (ADR-011) |

These mechanisms protect their stated surfaces, not every local/server record.
Plaintext backups, local caches, identities and operational metadata remain
separate exposures.

---

## 6. What the vault does NOT protect against

These include inherent limits and current implementation boundaries. They must not
be mistaken for guarantees provided by the vault format.

- **Keyloggers or screen capture on your device.** If malware records your keystrokes or
  screen, it can capture your master password or read decrypted data straight from the UI.
  Cryptography cannot help once the endpoint is watching you.
- **A compromised app binary (supply-chain attack).** If the ldgr binary you run has been
  tampered with — malicious build, poisoned dependency, backdoored update or modified
  web-delivered code — it can exfiltrate keys or plaintext directly. The vault format
  assumes the code decrypting it is honest.
- **A compromised or rooted operating system.** Root-level access can read process memory,
  intercept syscalls, or harvest cached keys (including a biometric-unlock vault session key in the OS
  keychain). Application-level encryption cannot defeat a hostile OS (see §4.6).
- **Plaintext copies and derived caches.** Migration retains a plaintext backup.
  Local labels, native widget/intent data and watch summaries are not vault-encrypted.
  Native database encryption and sealed web ledger persistence do not automatically
  protect those copies or separately stored sync credentials.
- **Memory, swap and hibernation capture.** Key-type zeroization and formatted
  redaction cannot protect live decryption keys/pages or erase all copied buffers;
  SQLCipher memory locking is not enabled.
- **Rubber-hose cryptanalysis.** Coercion, legal compulsion, or extortion to reveal your
  password or recovery key is outside any cryptographic defense.
- **Lost password *and* lost recovery key — unrecoverable by design.** There is no master
  key, no back door, and no reset path on ldgr's side. If both secrets are lost, the data
  is permanently unreadable. This is the deliberate cost of nobody else being able to read
  it: a back door for you would be a back door for everyone.
- **Metadata leakage and operational logs.** Canonical batch padding hides exact
  plaintext lengths, not account identities, device descriptors, upload counts,
  hashes or activity timing. Session records reveal login/session activity, not a
  reliable physical-device count. See the [privacy inventory](./metadata-and-privacy.md)
  for readable server fields, logging limits and retention.

---

## 7. Argon2id parameter rationale

The master password is stretched with **Argon2id** (`Algorithm::Argon2id`,
`Version::V0x13`) into the 256-bit Master Key (`kdf.rs`). Argon2id is memory-hard:
attacker cost scales with both time **and** memory, which blunts GPU/ASIC brute-forcing far
better than iteration-only KDFs. Parameters are chosen per platform to balance brute-force
resistance against an acceptable unlock latency on real hardware:

| Profile | Memory | Iterations | Parallelism | Rationale |
|---------|--------|-----------|-------------|-----------|
| **Desktop** | 256 MB | 3 | 4 | Ample RAM and cores → maximize memory hardness, the strongest lever against parallel attackers |
| **Mobile** | 64 MB | 4 | 2 | Less RAM and stricter latency budgets → trade memory down, raise iterations to partially compensate |
| **WASM** | 64 MB | 3 | 1 | Browser threading is limited, so single-threaded; memory bounded to keep page download/init reasonable |
| **Test** | 64 KB | 1 | 1 | **Fast, deliberately weak — never for production.** Used only so the test suite runs quickly |

The chosen parameters (salt + `argon2_params`) are stored **in the header** so a vault
created on one device unlocks on another. Because they are explicit and versioned
(`kdf_version`), they can be **upgraded over time**: on password change, ldgr re-derives
with stronger parameters, so vaults strengthen as hardware improves. The KDF validates a
minimum floor (≥ 8 KiB memory, ≥ 1 iteration, ≥ 1 lane) to reject obviously broken inputs.

> **Note on the test profile.** The 64 KB / 1 / 1 parameters provide essentially no
> brute-force resistance. They exist solely to keep automated tests fast and must never be
> selected for a real vault. A downgrade of a production vault's parameters to test-level is
> treated as a security issue (see `SECURITY.md` — "Argon2id parameter downgrade attacks").

---

## 8. Side-channel considerations

- **Memory hygiene.** Core key types (`MasterKey`, `AuthKey`, `MasterEncryptionKey`,
  `VaultKey`, `ItemKey`, `RecoveryKey`, `DatabaseKey`) derive `Zeroize` and `ZeroizeOnDrop` (`keys.rs`),
  so their bytes are overwritten when they leave scope rather than lingering in freed
  memory. This does not extend automatically to exported copies, every plaintext buffer
  or SQLCipher's page cache.
- **Debug redaction.** Each key type has a hand-written `Debug` impl that prints
  `[REDACTED]`, with tests asserting no byte or hex value leaks. This prevents accidental
  exposure through those types' formatted output, not through raw memory/crash images
  or every log-producing surface.
- **Primitive implementations.** Envelope encryption and key derivation use
  RustCrypto implementations (`aes-gcm`, `argon2`, `hkdf`). That choice alone is
  not a guarantee that all session, database and platform handling is constant-time
  or free of other side channels.
- **Authentication failures are uniform.** Wrap/unwrap and seal/open paths return a generic
  `CryptoError` on any GCM tag mismatch and never include key material in the message, so an
  attacker learns only "decryption failed," not *why*.
- **Session-key caching caveat.** To support biometric unlock, an unlocked session key can
  be exported and later restored (`export_session_key` / `restore_vault_from_session`).
  When used, protection depends on the OS keystore/Keychain and its access controls.
  The cached material is the vault session key, not the password-derived MEK.
- **Padding is coarse, not perfect.** Size-bucket padding hides exact lengths but not bucket
  boundaries, envelope/blob counts, or timing (restated from §3/§6 because it is a genuine residual
  channel).

---

## 9. Future improvements

Stated openly so reviewers know the current boundaries:

- **Container-level HMAC over the full header.** Today the wrapped keys and encrypted
  metadata are individually authenticated by their own GCM tags, but the **plaintext header
  parameters** (`format_version`, `kdf_version`, `salt`, `argon2_params`) are only
  *indirectly* authenticated: tampering with the salt or KDF parameters changes the derived
  MEK, which then fails to unwrap the Vault Key. This reliably prevents a *successful*
  unlock with altered parameters, but it does not let the client distinguish header
  corruption from a wrong password, and it offers no cryptographic binding of the header as
  a unit. A dedicated MAC (or AAD binding) over the entire header would authenticate these
  fields directly and enable clearer error reporting and downgrade detection.
- **Stronger metadata-leakage defenses.** Optional cover traffic or fixed-cadence sync could
  reduce the timing/count signal noted in §6, at a bandwidth cost.
- **Data minimization beyond the format.** Local caches, retained copies, device
  descriptors and operational logs need their own protection and retention decisions;
  envelope encryption alone does not provide them.

---

## 10. Summary: threat → mitigation → residual risk

| Threat actor / surface | In scope? | Mitigation | Residual risk |
|------------------------|-----------|-----------|----------------|
| Curious server / sync operator | ✅ Yes (financial ciphertext) | Client-side AES-256-GCM; SRP-6a does not send the password | Plaintext identities/auth records, device descriptors, blob paths/counts/size ranges/hashes/timing and personal data in logs |
| Network attacker (MITM) | ✅ Yes (encrypted payloads) | Financial payloads encrypted before transport; TLS protects credentials/metadata; GCM tags detect ciphertext modification | Traffic analysis; drop/delay; AEAD alone does not prove freshness |
| Device thief — locked device / disk image | ✅ Yes (encrypted stores) | Argon2id-wrapped VK; encrypted vault container; native SQLCipher store; sealed web exports | Weak passwords, accessible cached keys, retained plaintext backup, local labels and derived caches |
| Forensic examiner — memory / swap dump | ⚠️ Partial | Core key-type zeroization and formatted redaction; application lock/expiry handling | Live keys/pages and copied buffers; no SQLCipher memory locking; swap/hibernation; no universal immediate-erasure deadline |
| Another tenant on a shared sync server | ✅ Yes | Account-scoped vault lookups (404 for unowned vaults); random newly minted identifiers; taken requested identifiers replaced (ADR-011) | Legacy identifiers need not be random; shared-resource consumption |
| Malicious co-resident / rooted OS | ❌ No | Relies on OS process/sandbox boundary | Full compromise if the OS is hostile |
| Keylogger / screen capture | ❌ No | None (endpoint trust assumed) | Password and plaintext fully exposed |
| Compromised app binary (supply chain) | ❌ No | Reproducible builds / signing are process controls, not format guarantees | Malicious binary can exfiltrate keys/plaintext |
| Rubber-hose / coercion | ❌ No | None | User compelled to reveal secrets |
| Lost password + lost recovery key | ❌ No (by design) | No back door, no master key, no reset | Data permanently unrecoverable |
| Metadata analysis (identity/sizes/counts/timing) | ⚠️ Partial | Size-bucket padding of canonical envelopes; TLS hides request content from passive network observers | Server retains readable identity/activity relationships; padding does not hide blob count or timing |
| Logs, retention and historical copies | ❌ No (format does not govern them) | Operator-defined access controls, collection and retention policy | Personal data and potentially sensitive request metadata; expiry is not deletion; logs/backups may outlive current rows |

---

## 11. References

- **ADR-001** — Source of truth model: [`docs/adr/001-source-of-truth.md`](../adr/001-source-of-truth.md)
- **ADR-003** — Sync & conflict resolution: [`docs/adr/003-sync-conflict-resolution.md`](../adr/003-sync-conflict-resolution.md)
- **ADR-004** — Data model: [`docs/adr/004-data-model.md`](../adr/004-data-model.md)
- **ADR-005** — Platform boundaries: [`docs/adr/005-platform-boundaries.md`](../adr/005-platform-boundaries.md)
- **ADR-006** — Licensing (server/AGPL boundary): [`docs/adr/006-licensing.md`](../adr/006-licensing.md)
- **ADR-010** — Per-platform working-store encryption: [`docs/adr/010-local-working-store-at-rest-encryption.md`](../adr/010-local-working-store-at-rest-encryption.md)
- **ADR-011** — Tenant-scoped vault identifiers: [`docs/adr/011-tenant-scoped-vault-identifiers.md`](../adr/011-tenant-scoped-vault-identifiers.md)
- **Architecture** — §4 Encryption Architecture, §10 Sync Server: [`docs/ldgr-architecture.md`](../ldgr-architecture.md)
- **Plain-English overview**: [`docs/security/vault-overview.md`](./vault-overview.md)
- **Metadata and privacy inventory**: [`docs/security/metadata-and-privacy.md`](./metadata-and-privacy.md)
- **Security policy & disclosure**: [`SECURITY.md`](../../SECURITY.md)
- **Implementation:**
  - Key derivation & Argon2 params: [`crates/ldgr-core/src/crypto/kdf.rs`](../../crates/ldgr-core/src/crypto/kdf.rs)
  - Key types, `Zeroize`, `Debug` redaction: [`crates/ldgr-core/src/crypto/keys.rs`](../../crates/ldgr-core/src/crypto/keys.rs)
  - Key wrapping & AAD domain separation: [`crates/ldgr-core/src/crypto/wrap.rs`](../../crates/ldgr-core/src/crypto/wrap.rs)
  - Per-item envelope encryption & padding: [`crates/ldgr-core/src/crypto/envelope.rs`](../../crates/ldgr-core/src/crypto/envelope.rs)
  - Vault header, unlock, recovery: [`crates/ldgr-core/src/crypto/vault.rs`](../../crates/ldgr-core/src/crypto/vault.rs)
  - Recovery key handling: [`crates/ldgr-core/src/crypto/recovery.rs`](../../crates/ldgr-core/src/crypto/recovery.rs)
