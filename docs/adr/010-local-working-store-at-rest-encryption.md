# ADR-010: Local Working-Store At-Rest Encryption (Per-Platform Model)

**Status**: Accepted  
**Date**: 2026-07-27  
**Decision makers**: @kafkade  

## Context

A vault is a sealed, zero-knowledge container: the `.ldgr` file and every sync blob are
AES-256-GCM encrypted, and the server never sees plaintext (ADR-001, ADR-004). But to *use* a
vault, each platform decrypts it into a working **SQLite** database and runs queries against
that live store. If that working store is written to disk in plaintext, an attacker with
filesystem access to a **locked** vault (another local user, a stolen locked laptop, an iOS
backup, a browser profile on shared hardware) can read account names, transaction descriptions,
amounts, and commodities directly — defeating the point of encrypting the vault at rest.

Two efforts closed this gap:

- **#295** encrypted the CLI working store with SQLCipher and moved the unlocked session key out
  of a plaintext `session.json` into the OS keystore. It added the reusable core primitives
  (`crypto::derive_db_key`, `crypto::DatabaseKey::to_pragma_hex`) and the `ldgr migrate` command.
- **#315** (this ADR) extends the same at-rest guarantee to the **iOS/FFI** path and formally
  documents that the **web/WASM** path was already encrypted at rest by design.

The platforms have materially different storage substrates, so a single mechanism does not fit
all of them. This ADR records the **per-platform** model and why each choice is correct.

### Constraints

- `ldgr-core` is zero-I/O and WASM-safe (ADR-005). The key-derivation primitive lives in core;
  all file I/O (opening the store, SQLCipher `PRAGMA`, migration) lives in the platform crates
  (`ldgr-cli`, `ldgr-ffi`) — never in core.
- The `core` WASM bundle must stay < 2 MB gzip (ADR-005). rusqlite/SQLCipher/OpenSSL must **not**
  enter the WASM dependency graph.
- watchOS cannot build vendored OpenSSL: `openssl-src` cannot configure OpenSSL for
  `aarch64-apple-watchos`.

## Decision

### 1. Derive the database key from the vault key, in core

All keyed platforms use the same subkey derivation, exposed by `ldgr-core`:

```
VaultKey ── HKDF-SHA256(info = "ldgr-sqlcipher-key-v1") ──▶ DatabaseKey (32 bytes)
```

`DatabaseKey::to_pragma_hex()` renders the raw-key SQLCipher form `x'<64 hex>'`, so SQLCipher
uses the derived bytes directly and skips its own PBKDF2 (we already derive from the Argon2id
key chain). `DatabaseKey` is `Zeroize`/`ZeroizeOnDrop` with a redacted `Debug`. The working
store is therefore keyed by the vault key and can only be opened by an unlocked vault — never by
password alone at the file level, and never by the server.

### 2. Per-platform at-rest mechanism

| Platform | Working store | At-rest mechanism |
|----------|---------------|-------------------|
| **CLI** (`ldgr-cli`) | `vault.db` on disk | SQLCipher keyed file (`PRAGMA key`), rusqlite `bundled-sqlcipher-vendored-openssl` |
| **iOS/iPadOS** (`ldgr-ffi`) | `vault.db` on device | SQLCipher keyed file (`PRAGMA key`), same derivation as CLI |
| **watchOS** (`ldgr-ffi`) | *none* | **N/A** — the watch app never opens a vault DB (see Decision 4) |
| **Web** (`ldgr-wasm`) | in-memory `sql.js` | No on-disk DB at all; only the **sealed vault container** is persisted (see Decision 3) |

For CLI and iOS the keyed open is a single shared shape (`ldgr-cli`'s `db::open_encrypted`,
`ldgr-ffi`'s `open_encrypted_db`): open the connection, apply the raw-key `PRAGMA key`, then
force a page read (`SELECT count(*) FROM sqlite_master`) so a wrong key or an unmigrated
plaintext store fails immediately with a clear error rather than at the first query.

We deliberately do **not** enable `PRAGMA cipher_memory_security`. #295 removed it because its
process-global `mlock`/`VirtualLock`-on-every-allocation hook exhausts the small default Windows
working-set quota (surfacing as a spurious `STATUS_STACK_OVERFLOW`). At-rest confidentiality
comes entirely from `PRAGMA key` encrypting the file on disk, not from locking in-memory pages.

### 3. Web is encrypted at rest *by construction*, not by SQLCipher

The web app runs SQLite via `sql.js`, which is **in-memory only** (the database lives in the
WASM heap and is never written to disk). Persistence works differently and was designed this way
from the start (the "hold-in-memory + re-seal" model):

1. `db.export()` produces the plaintext sql.js bytes **in memory**.
2. Those bytes are placed into the vault as an item and `serializeVault()` seals the whole vault
   with AES-256-GCM (the core container crypto).
3. Only the **sealed** vault blob is written to IndexedDB (`ldgr-vault/vaults`).

The plaintext `db.export()` bytes never reach disk — they are always wrapped inside the sealed
container first. The sync session token and the 2SKD Account Secret Key live in the sql.js
`sync_state` table, so they are sealed along with the ledger. `localStorage` holds only the
non-secret theme preference; `sessionStorage` holds only the ephemeral admin-panel bearer token
(in-memory session, cleared on sign-out/tab close, deliberately never persisted). There is no
Cache API / OPFS / filesystem write path for ledger data.

**Consequence:** the web path must **not** add rusqlite/SQLCipher/OpenSSL — doing so would break
the WASM budget (ADR-005) for zero at-rest benefit, since nothing plaintext is persisted. A
regression test asserts the persisted IndexedDB blob is ciphertext (no `SQLite format 3` magic,
no known plaintext tokens) and that `localStorage`/`sessionStorage` carry no vault secrets.

### 4. watchOS is out of scope for at-rest (no vault on the watch)

The watch app links only `LdgrShared` and receives pre-computed summaries over
WatchConnectivity; it never imports `LdgrSwift`/`LdgrFFI` and never opens a vault database. There
is no working store on the watch to encrypt. Because `openssl-src` cannot cross-compile for
`aarch64-apple-watchos`, `ldgr-ffi` scopes its rusqlite features **per target**: non-watchOS
Apple/desktop targets use `bundled-sqlcipher-vendored-openssl`; the watchOS slice uses plain
`bundled` SQLite (kept in the XCFramework, but it never keys or opens a store).

> ⚠️ On plain SQLite the `PRAGMA key` is **silently ignored** and the store would be written in
> plaintext. The SQLCipher feature must therefore be genuinely enabled on every target that
> actually opens a vault (CLI, iOS device/simulator, macOS). This is enforced by the per-target
> feature scoping above plus an at-rest byte-level test.

### 5. Migration is explicit and reversible, on every keyed platform

Legacy plaintext `vault.db` files from prior versions are upgraded by the same procedure on CLI
and iOS: detect the 16-byte `SQLite format 3\0` header → `ATTACH` a freshly keyed database and
`sqlcipher_export` into it → verify the copy preserves the schema version and every table's row
count → atomically swap it in, keeping the original as `vault.db.plaintext.bak` for backout.

Migration is **user-driven**, never silent: the CLI's `ldgr unlock` only *detects* and instructs;
`ldgr migrate` performs it. On iOS the FFI exposes `needs_migration()` (header check, no unlock)
plus `migrate(password)` and `migrate_with_session_key(key)` (biometric-upgrade path); the app
runs migration explicitly before opening. Web needs no migration — it was never plaintext at rest.

## Consequences

### Positive

- The at-rest guarantee now holds on CLI, iOS/iPadOS, and web with a single shared key
  derivation and a documented, testable model per platform.
- The WASM budget is preserved: web stays SQLCipher-free because it never needs it.
- Migration is safe (verified, reversible) and never surprises the user.

### Negative / trade-offs

- iOS device/simulator/macOS builds now compile vendored OpenSSL + SQLCipher, increasing FFI
  build time and requiring explicit Apple deployment targets at link time
  (`IPHONEOS_DEPLOYMENT_TARGET`/`MACOSX_DEPLOYMENT_TARGET`) so vendored-OpenSSL objects that
  reference `___chkstk_darwin` link cleanly.
- The at-rest mechanism is intentionally **not uniform** across platforms (keyed SQLCipher file
  vs. sealed in-memory container). This ADR is the reference for why that asymmetry is correct.

## Related

- ADR-001 (source of truth), ADR-004 (data model), ADR-005 (platform boundaries / WASM budget),
  ADR-008 (2SKD Secret Key lives in `sync_state`, so it is sealed with the ledger).
- Issues #295 (CLI at-rest + core primitives + `ldgr migrate`) and #315 (iOS/FFI + web
  verification).
- This corresponds to the private security assessment's draft at-rest-encryption ADR
  (numbered 013 in that draft); the repo's next-sequential number is 010.
