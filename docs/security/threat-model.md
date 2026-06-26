# Vault format threat model

> **Who this is for:** security-conscious users, auditors, and engineers who want a
> precise account of what ldgr's vault encryption protects, what it does not, and why.
>
> For a plain-English overview with no technical background required, see
> [How is my data protected?](./vault-overview.md). For the full system design, see the
> [ldgr Architecture](../ldgr-architecture.md) (§4 Encryption Architecture, §10 Sync
> Server). This document is the authoritative, expanded version of the threat-model
> summary in architecture §4.5.

ldgr is a **zero-knowledge, local-first** personal finance app. Financial data is
encrypted on your device before it is ever written to disk or sent over the network. The
server — including ldgr's own — only ever sees opaque ciphertext. This document states the
guarantees that claim rests on, and is deliberate about its limits: **honest disclosure of
what we do not protect against builds more trust than overclaiming.**

All facts below are grounded in the implementation under
[`crates/ldgr-core/src/crypto/`](../../crates/ldgr-core/src/crypto/). No custom
cryptography is used; primitives come from the audited
[RustCrypto](https://github.com/RustCrypto) crates.

---

## 1. Scope

**In scope:** the confidentiality, integrity, and authenticity of vault data at rest (on
disk), in transit (during sync), and — on a best-effort basis — in memory while the vault
is unlocked.

**Out of scope:** the security of the device's operating system, the integrity of the app
binary itself, and any attack that observes the user directly (their screen, keyboard, or
person). These are explicitly enumerated in [§6](#6-what-the-vault-does-not-protect-against)
rather than silently assumed away.

---

## 2. Asset inventory

| Asset | Where it lives | How it is protected |
|-------|----------------|---------------------|
| **Financial transaction data** (accounts, postings, balances, budgets) | Encrypted items inside the vault file; synced as encrypted blobs | Per-item AES-256-GCM under a random item key (`envelope.rs`) |
| **Master password** | User's memory only | Never stored, never transmitted; only an Argon2id-derived key leaves the KDF (`kdf.rs`) |
| **Master Key (MK)** | Memory only, during an unlocked session | Derived on unlock, never persisted; `Zeroize`/`ZeroizeOnDrop` (`keys.rs`) |
| **Master Encryption Key (MEK)** | Memory only (optionally cached in OS keychain for biometric unlock) | HKDF-derived from MK; zeroized on drop |
| **Vault Key (VK)** | Stored **wrapped** in the header; unwrapped into memory on unlock | AES-256-GCM-wrapped by MEK *and* by the recovery key (`wrap.rs`) |
| **Per-item keys (IK)** | Stored **wrapped** alongside each item | AES-256-GCM-wrapped by VK; one random key per item |
| **Recovery key** | Displayed once to the user as an emergency kit; not stored by ldgr | 256-bit random; user stores it offline. Wraps the VK independently of the password |
| **Vault metadata** (vault name, `created_at`) | Encrypted blob in the header (`encrypted_metadata`) | AES-256-GCM under VK — **not** readable without unlocking |
| **Header KDF parameters** (`salt`, `argon2_params`, `format_version`, `kdf_version`) | **Plaintext** in the header (`VaultHeader`) | Parseable without the password; only *indirectly* authenticated (see [§9](#9-future-improvements)) |

A useful consequence: an attacker holding the raw vault file can read the Argon2id
parameters and salt (needed to attempt unlock) but **cannot** read the vault's name,
creation time, or any financial data.

---

## 3. Security properties

The vault provides three properties, each backed by a specific mechanism:

- **Confidentiality.** Every item payload is encrypted with AES-256-GCM under a unique,
  random per-item key (`encrypt_item` in `envelope.rs`). Item keys are wrapped by the
  Vault Key; the Vault Key is wrapped by the password-derived MEK. Without the password
  (or recovery key), nothing in the item layer is recoverable.

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

- **Length hiding (partial).** Item payloads are padded to size buckets
  (512 B / 2 KB / 8 KB / 32 KB, then 32 KB multiples) before encryption (`envelope.rs`),
  so an observer cannot infer the exact size — and therefore the complexity — of a
  transaction from its ciphertext. This does **not** hide item *count* or *timing*.

---

## 4. Threat actors

Each actor below is given an explicit **in-scope** (the design defends against it) or
**out-of-scope** (acknowledged, not defended) classification.

### 4.1 Curious server / sync operator — **in scope**

*Capabilities:* full read/write access to every stored blob, the header, sync metadata,
and SRP-6a authentication records. This includes ldgr's own hosted server and any
self-hosted or third-party blob store.

*Analysis:* the server is an encrypted blob store that never holds a decryption key. It
authenticates clients via **SRP-6a** (RFC 5054), so the password is never sent — the
server stores only a salt and verifier, never anything that can derive the MEK
(architecture §10). All item data, item keys, the Vault Key, and even the vault name are
encrypted client-side before upload. A malicious operator sees ciphertext, blob sizes
(bucketed), blob counts, and sync timing — and nothing else. **The operator cannot read
financial data.** Residual exposure: traffic-analysis metadata (how many items exist, when
you sync). See ADR-003 and ADR-006.

### 4.2 Network attacker (MITM on the sync transport) — **in scope**

*Capabilities:* intercept, replay, or modify traffic between the client and sync server.

*Analysis:* data is already AES-256-GCM-encrypted **before** it reaches the transport, so
interception yields only ciphertext — TLS is defense-in-depth, not the sole protection.
Tampering is caught two ways: TLS at the transport layer, and the per-blob GCM auth tag at
the application layer (a flipped bit in any blob fails authentication on decrypt). A MITM
cannot read or silently alter financial data. Residual exposure: the same traffic-analysis
metadata as the server actor; an active attacker can also drop/delay traffic (denial of
service), which is an availability concern, not a confidentiality one.

### 4.3 Device thief with a locked device or disk image — **in scope (at rest)**

*Capabilities:* physical possession of a powered-off/locked device, or a forensic copy of
its disk, including the raw vault file.

*Analysis:* at rest the vault is only as strong as the password feeding Argon2id. The thief
can parse the plaintext header (salt + params) and mount an **offline** guessing attack,
but each guess costs a full Argon2id evaluation (hundreds of MB of memory, multiple
iterations — see [§7](#7-argon2id-parameter-rationale)), making large-scale brute force
expensive. With a strong, high-entropy password the data is effectively unreadable. This
actor moves **out of scope** if the device was captured *unlocked* with the vault already
open (see §4.4 and §6).

### 4.4 Forensic examiner with a memory dump or swap file — **partial / best-effort**

*Capabilities:* a RAM image or swap/hibernation file captured while the vault was unlocked,
or shortly after.

*Analysis:* while unlocked, the MK, MEK, VK, and any in-use item keys necessarily exist in
plaintext in memory — this is unavoidable for any app that decrypts data. ldgr reduces the
window and residue: every key type implements `Zeroize`/`ZeroizeOnDrop` so material is
wiped when dropped (`keys.rs`), the session locks and evicts keys after an idle timeout
(architecture §4.4), and `Debug` formatting redacts all key bytes to `[REDACTED]` so
secrets cannot leak into logs or crash dumps. These are **best-effort** mitigations: they
cannot defeat a dump taken at the exact moment keys are live, and the OS may page memory to
swap outside ldgr's control. Classified partial, honestly.

### 4.5 Malicious app co-resident on the same device — **out of scope**

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
| In transit / server | **Server compromise** | Server only ever stores ciphertext; SRP-6a means it never receives the password; no server-side key exists to decrypt with |
| In transit | **Network interception** | Client-side AES-256-GCM applied before the transport; TLS as a second layer; GCM tags detect tampering |
| At rest | **At-rest exposure** (stolen disk/file) | Argon2id-derived MEK wraps the Vault Key; all items AES-256-GCM-encrypted; metadata encrypted too |
| At rest | **Offline brute force** | Argon2id memory-hard KDF makes each password guess costly; per-platform parameters tuned for resistance (§7) |

In all four cases the attacker is left with authenticated ciphertext and, at most,
coarse-grained metadata (bucketed sizes, item counts, sync timing).

---

## 6. What the vault does NOT protect against

These limitations are inherent, not oversights. Stating them plainly is part of the
security model.

- **Keyloggers or screen capture on your device.** If malware records your keystrokes or
  screen, it can capture your master password or read decrypted data straight from the UI.
  Cryptography cannot help once the endpoint is watching you.
- **A compromised app binary (supply-chain attack).** If the ldgr binary you run has been
  tampered with — malicious build, poisoned dependency, backdoored update — it can exfiltrate
  keys or plaintext directly. The vault format assumes the code decrypting it is honest.
- **A compromised or rooted operating system.** Root-level access can read process memory,
  intercept syscalls, or harvest cached keys (including a biometric-unlock MEK in the OS
  keychain). Application-level encryption cannot defeat a hostile OS (see §4.5).
- **Rubber-hose cryptanalysis.** Coercion, legal compulsion, or extortion to reveal your
  password or recovery key is outside any cryptographic defense.
- **Lost password *and* lost recovery key — unrecoverable by design.** There is no master
  key, no back door, and no reset path on ldgr's side. If both secrets are lost, the data
  is permanently unreadable. This is the deliberate cost of nobody else being able to read
  it: a back door for you would be a back door for everyone.
- **Metadata leakage.** Padding hides exact payload sizes, but an observer of the sync
  store still learns approximate item counts, bucket sizes, and *when* you sync — which can
  hint at activity patterns. This is acknowledged as a partial gap, not a full mitigation.

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

- **Memory hygiene.** All six key types (`MasterKey`, `AuthKey`, `MasterEncryptionKey`,
  `VaultKey`, `ItemKey`, `RecoveryKey`) derive `Zeroize` and `ZeroizeOnDrop` (`keys.rs`),
  so their bytes are overwritten when they leave scope rather than lingering in freed
  memory.
- **Debug redaction.** Each key type has a hand-written `Debug` impl that prints
  `[REDACTED]`, with tests asserting no byte or hex value leaks. This prevents accidental
  exposure through logs, panics, or error formatting.
- **No secret-dependent branching in ldgr.** ldgr writes no cryptographic primitives of its
  own. Constant-time behavior of AES-256-GCM, Argon2id, HKDF-SHA256, and X25519 is
  delegated to the audited RustCrypto crates (`aes-gcm`, `argon2`, `hkdf`,
  `x25519-dalek`). `unsafe` code is forbidden in the crate.
- **Authentication failures are uniform.** Wrap/unwrap and seal/open paths return a generic
  `CryptoError` on any GCM tag mismatch and never include key material in the message, so an
  attacker learns only "decryption failed," not *why*.
- **Session-key caching caveat.** To support biometric unlock, an unlocked session key can
  be exported and later restored (`export_session_key` / `restore_vault_from_session`).
  When used, the key's security becomes bounded by the OS keychain/Secure Enclave holding
  it — a deliberate convenience-vs-exposure tradeoff, surfaced here for transparency.
- **Padding is coarse, not perfect.** Size-bucket padding hides exact lengths but not bucket
  boundaries, item counts, or timing (restated from §3/§6 because it is a genuine residual
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

---

## 10. Summary: threat → mitigation → residual risk

| Threat actor / surface | In scope? | Mitigation | Residual risk |
|------------------------|-----------|-----------|----------------|
| Curious server / sync operator | ✅ Yes | Client-side AES-256-GCM; SRP-6a (no password to server); zero-knowledge blob store | Coarse metadata: item count, bucketed sizes, sync timing |
| Network attacker (MITM) | ✅ Yes | Data encrypted before transport + TLS; per-blob GCM auth tags detect tampering | Traffic analysis; availability (drop/delay) |
| Device thief — locked device / disk image | ✅ Yes (at rest) | Argon2id-wrapped Vault Key; all items + metadata encrypted | Offline guessing of weak passwords; out of scope if captured unlocked |
| Forensic examiner — memory / swap dump | ⚠️ Partial | `Zeroize`/`ZeroizeOnDrop`, idle auto-lock, `Debug` redaction | Keys live in RAM while unlocked; OS may page to swap |
| Malicious co-resident / rooted OS | ❌ No | Relies on OS process/sandbox boundary | Full compromise if the OS is hostile |
| Keylogger / screen capture | ❌ No | None (endpoint trust assumed) | Password and plaintext fully exposed |
| Compromised app binary (supply chain) | ❌ No | Reproducible builds / signing are process controls, not format guarantees | Malicious binary can exfiltrate keys/plaintext |
| Rubber-hose / coercion | ❌ No | None | User compelled to reveal secrets |
| Lost password + lost recovery key | ❌ No (by design) | No back door, no master key, no reset | Data permanently unrecoverable |
| Metadata analysis (sizes/counts/timing) | ⚠️ Partial | Size-bucket padding | Item count, bucket size, and timing still observable |

---

## 11. References

- **ADR-001** — Source of truth model: [`docs/adr/001-source-of-truth.md`](../adr/001-source-of-truth.md)
- **ADR-003** — Sync & conflict resolution: [`docs/adr/003-sync-conflict-resolution.md`](../adr/003-sync-conflict-resolution.md)
- **ADR-004** — Data model: [`docs/adr/004-data-model.md`](../adr/004-data-model.md)
- **ADR-005** — Platform boundaries: [`docs/adr/005-platform-boundaries.md`](../adr/005-platform-boundaries.md)
- **ADR-006** — Licensing (server/AGPL boundary): [`docs/adr/006-licensing.md`](../adr/006-licensing.md)
- **Architecture** — §4 Encryption Architecture, §10 Sync Server: [`docs/ldgr-architecture.md`](../ldgr-architecture.md)
- **Plain-English overview**: [`docs/security/vault-overview.md`](./vault-overview.md)
- **Security policy & disclosure**: [`SECURITY.md`](../../SECURITY.md)
- **Implementation:**
  - Key derivation & Argon2 params: [`crates/ldgr-core/src/crypto/kdf.rs`](../../crates/ldgr-core/src/crypto/kdf.rs)
  - Key types, `Zeroize`, `Debug` redaction: [`crates/ldgr-core/src/crypto/keys.rs`](../../crates/ldgr-core/src/crypto/keys.rs)
  - Key wrapping & AAD domain separation: [`crates/ldgr-core/src/crypto/wrap.rs`](../../crates/ldgr-core/src/crypto/wrap.rs)
  - Per-item envelope encryption & padding: [`crates/ldgr-core/src/crypto/envelope.rs`](../../crates/ldgr-core/src/crypto/envelope.rs)
  - Vault header, unlock, recovery: [`crates/ldgr-core/src/crypto/vault.rs`](../../crates/ldgr-core/src/crypto/vault.rs)
  - Recovery key handling: [`crates/ldgr-core/src/crypto/recovery.rs`](../../crates/ldgr-core/src/crypto/recovery.rs)
