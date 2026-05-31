# ldgr — Architecture, Plan & Roadmap

> **Zero-knowledge bookkeeping.**
> An open-source, privacy-first personal finance system built on plain-text accounting principles.

---

## Table of Contents

1. [Project Identity](#1-project-identity)
2. [Architecture Decision Records](#2-architecture-decision-records)
3. [System Architecture](#3-system-architecture)
4. [Zero-Knowledge Architecture](#4-zero-knowledge-architecture)
5. [Feature Architecture](#5-feature-architecture)
6. [Data Model](#6-data-model)
7. [Platform Design](#7-platform-design)
8. [Repository Structure](#8-repository-structure)
9. [Technology Choices](#9-technology-choices)
10. [Phased Roadmap](#10-phased-roadmap)
11. [Risk Register](#11-risk-register)
12. [Anti-Requirements](#12-anti-requirements)
13. [Open Questions](#13-open-questions-requiring-prototyping)

---

## 1. Project Identity

| Field | Value |
|-------|-------|
| **Name** | ldgr |
| **Backup name** | tallyx |
| **Tagline** | Zero-knowledge bookkeeping |
| **GitHub** | `github.com/kafkade/ldgr` |
| **Crate** | `ldgr` (crates.io) |
| **npm** | `ldgr` (npmjs.com) |
| **License** | Apache-2.0 (all components except sync server: AGPL-3.0) |
| **Target audience** | Privacy-conscious power users, plain-text accounting community |

**Pronunciation**: "ledger" — the name is "ledger" with vowels removed, in the tradition of cryptic/A24-style naming.

### Positioning Statement

ldgr is a local-first, encrypted-vault personal finance system that combines the rigor of double-entry bookkeeping (hledger-compatible) with the polish of consumer finance apps, the market-tracking capabilities of terminal tools like ticker/tickrs, and a zero-knowledge encryption architecture where no server — including its own — ever sees plaintext financial data.

---

## 2. Architecture Decision Records

### ADR-001: Source of Truth — Vault-Canonical

**Status**: Accepted

**Context**: There is fundamental tension between plain-text accounting (human-readable journal files) and encrypted vault storage (structured, encrypted database).

**Decision**: The encrypted vault is the single source of truth. hledger journal format is the import/export interchange format, not the canonical store.

**Rationale**:
- Item-level encryption requires structured data — you can't encrypt individual transactions within a flat text file
- SQLite queries on mobile/Watch are fast; re-parsing journal text on every operation is wasteful
- Sync and merge on structured items with unique IDs is tractable; text-file merging is git-merge territory
- hledger compatibility is maintained through high-fidelity import (with defined subset) and one-way export

**Compatibility boundary** (refined after critique):
- **Import**: Supports a documented strict subset of hledger syntax. Unsupported features (includes, automated transactions, periodic transactions, valuation expressions) cause a clear error at import time with guidance: *"This journal uses `include` directives. Flatten with `hledger print` first."*
- **Export**: One-way export to hledger journal format for reporting. Users can pipe to hledger: `ldgr export --format hledger | hledger balance`. This is NOT bidirectional sync.
- **Validation tool**: `ldgr validate journal.hledger` checks importability before committing to migration.
- **Marketing**: ldgr is "hledger-compatible accounting" — not "hledger with encryption."

**Consequences**:
- Plain-text purists who want `vim` + `hledger` workflow won't adopt ldgr as their primary tool — and that's fine
- Round-trip fidelity is lossy by design (include structure, comments, formatting are not preserved)
- Import contract must be versioned and documented

---

### ADR-002: hledger Integration — Rust-Native Parser (Strict Subset)

**Status**: Accepted

**Context**: Mobile/WASM platforms cannot shell out to the hledger Haskell binary. We need accounting logic in Rust.

**Decision**: Implement a Rust-native parser and reporting engine for a strict, documented subset of hledger's journal format. Use a parser combinator library (nom or winnow). Validate against hledger binary output in CI via differential testing.

**Supported subset (v1.0)**:
- Plain transactions with postings (date, payee/description, account, amount)
- Amount-less postings (auto-balance)
- Balance assertions (single-commodity)
- Account declarations
- Commodity declarations
- Price directives (`P`)
- Tags and comments on transactions/postings
- Transaction status (cleared `*`, pending `!`)
- Multi-currency amounts

**Explicitly NOT supported in v1.0** (documented, error on import):
- `include` directives
- Automated/periodic transactions (`=`, `~`)
- Multiple commodities in balance assertions
- Lot notation (`{cost}`, `@@ total`)
- Valuation expressions
- Inline math expressions
- Timedot format

**Validation strategy**:
1. **CI**: Generate canonical test journals → parse with both ldgr and hledger → compare structured output
2. **Differential fuzzing**: Property-based tests generating random valid journals in the supported subset
3. **hledger test suite**: Run the subset of hledger's own tests that exercise supported features

**Desktop hybrid path** (optional, not core):
- On desktop where hledger binary is available, offer `ldgr import --via-hledger journal.hledger` which shells out to `hledger print --output-format=json` and imports the JSON. This handles edge cases the Rust parser doesn't cover yet.

**Consequences**:
- Scope is manageable (~3-4 months for core parser + reports, not 12+)
- Mobile/WASM get full offline accounting without hledger dependency
- Feature parity with hledger is a non-goal; ldgr is its own tool
- Parser scope expands over time based on user demand

---

### ADR-003: Sync & Conflict Resolution — Transaction-Atomic Events with Conflict Detection

**Status**: Accepted

**Context**: Cross-device sync (CLI ↔ iPhone ↔ iPad ↔ Watch ↔ Web) must maintain zero-knowledge guarantees and accounting invariants. The initial rubber-duck critique identified that last-write-wins (LWW) on individual field-level events breaks double-entry atomicity.

**Decision**: Transaction-atomic event log with three-way merge and mandatory user review for conflicts.

#### Sync Transport (layered)

| Layer | Transport | Use Case |
|-------|-----------|----------|
| **Primary (MVP)** | User-provided blob store via API (Dropbox API, Google Drive API, S3-compatible, WebDAV) | Easy adoption, no server setup |
| **Secondary** | Self-hosted sync server (AGPL-3.0) | Advanced features, lower latency, total ordering |
| **Backup only** | iCloud Drive / file system | Vault backup (single encrypted file), NOT event sync |

**Why not iCloud Drive for event sync**: File coordination APIs are unreliable (delayed propagation, silent failures, concurrent write corruption). iCloud Drive is fine for backing up the entire vault as a single file, but not for coordinating thousands of small event blobs.

#### Event Model

Events are **transaction-atomic** — a single event captures the full state of a transaction (all postings). No partial updates.

```
Event {
  id: UUIDv7,              // time-ordered, globally unique
  device_id: DeviceID,     // originating device
  lamport_clock: u64,      // logical clock
  entity_type: EntityType, // Transaction | Account | Price | Budget | Goal | Loan
  entity_id: UUID,         // the entity being modified
  operation: Operation,    // Create | Update | Delete
  payload: EncryptedBlob,  // full entity state, encrypted with vault item key
  version: u32,            // schema version for this event type
}
```

#### Conflict Resolution

1. **Detection**: Vector clock per device. On sync, compare vector clocks to identify divergence.
2. **Non-conflicting**: Events touching different entities merge automatically (deterministic total order on Lamport clock → UUIDv7 → device_id).
3. **Conflicting** (same entity modified on multiple devices):
   - **DO NOT auto-resolve with LWW**. Double-entry transactions are atomic — partial field merges break accounting invariants.
   - Flag conflicts for user review: "You edited this transaction on your phone ($50 → Dining) and your laptop ($100 → Groceries). Pick one or merge manually."
   - Conflict UI shows both versions side-by-side with a diff.
   - Until resolved, the newest version is displayed with a conflict indicator.
4. **Post-merge validation**: After every sync, validate all double-entry invariants (transactions balance, balance assertions pass). If validation fails, flag for review.

#### Event Batching

Events are encrypted in **batches** (per-sync-session or daily chunks), not individually. This reduces:
- Encryption overhead (fewer IVs, tags, key wraps)
- File system pressure (hundreds of files → tens)
- Sync enumeration cost

Batch format: `[event1, event2, ...event_n]` → serialize → encrypt with vault key → store as single blob.

#### Event Log Compaction

- Every 1,000 events OR monthly (whichever comes first), create a **snapshot**: the full materialized state at that event number
- Snapshot is encrypted and stored alongside event batches
- New device sync: download latest snapshot + event batches since snapshot → replay
- Target: new device onboarding < 30 seconds for 10 years of data

#### Device Onboarding

1. Existing device generates ephemeral X25519 keypair
2. Displays QR code containing: public key + connection info (LAN IP or server relay URL)
3. New device scans QR, establishes encrypted channel (X25519 → shared secret → AES-256-GCM)
4. Existing device sends vault key (encrypted with shared secret) over the channel
5. New device derives MEK from master password, unwraps vault key, syncs

**Consequences**:
- No silent data loss from concurrent edits
- Slightly more user friction on conflicts (manual review) — but this is correct for financial data
- Batch encryption is less granular than per-event but dramatically more efficient
- Snapshot mechanism adds complexity but bounds sync time

---

### ADR-004: Data Model — SQLite-Canonical with Versioned History + Event Sync Layer

**Status**: Accepted

**Context**: Full event sourcing (canonical event log + materialized views) adds substantial complexity. The critique questioned whether the complexity-to-value ratio is justified.

**Decision**: **SQLite-canonical with versioned rows for audit history, plus a thin event sync layer.** This is simpler than full event sourcing but still supports conflict-free sync.

**Architecture**:
```
┌─────────────────────────────────────┐
│         SQLite Database             │
│                                     │
│  ┌─────────────────────────────┐    │
│  │  Canonical Tables           │    │
│  │  (accounts, transactions,   │    │
│  │   postings, lots, prices,   │    │
│  │   budgets, goals, loans)    │    │
│  └─────────────────────────────┘    │
│                                     │
│  ┌─────────────────────────────┐    │
│  │  Sync Outbox                │    │
│  │  (pending events to push)   │    │
│  └─────────────────────────────┘    │
│                                     │
│  ┌─────────────────────────────┐    │
│  │  Sync State                 │    │
│  │  (vector clock, last sync)  │    │
│  └─────────────────────────────┘    │
└─────────────────────────────────────┘
```

**How sync works**:
1. Local mutation → write to canonical tables + append event to sync outbox
2. On sync push: encrypt outbox events → upload batches to blob store/server
3. On sync pull: download new batches → decrypt → apply to canonical tables (with conflict detection)
4. Conflicts: flag in `sync_conflicts` table → present to user

**Why not full event sourcing**:
- SQLite is the query layer AND the canonical store — no materialized view rebuild
- History is preserved via versioned rows (`version` column + soft deletes) — not via event replay
- Schema migrations are standard SQLite migrations, not event schema evolution
- First sync uses snapshot (SQLite dump), not full event replay

**Consequences**:
- Much simpler implementation than event sourcing
- SQLite is battle-tested on every platform (iOS, WASM via sql.js, desktop)
- Audit trail via versioned rows, not event replay
- Sync layer is a thin wrapper, not a foundational architectural pattern

---

### ADR-005: Platform Boundaries — Maximalist Rust Core with Platform-Native Networking

**Status**: Accepted

**Context**: The critique identified concerns about WASM bundle size, UniFFI async maturity, and platform-native networking integration.

**Decision**: Maximalist Rust core for computation, but platform-native code handles networking (HTTP requests) and platform APIs.

#### Rust Core (`ldgr-core` crate)

| Module | Responsibility |
|--------|---------------|
| `crypto` | Key hierarchy, vault encryption/decryption, Argon2id, SRP-6a |
| `accounting` | Journal parser, double-entry validation, reports, queries |
| `storage` | SQLite schema, CRUD operations, versioned history |
| `sync` | Event generation, conflict detection, merge logic, snapshot creation |
| `import` | CSV/OFX/QFX/hledger parsing, deduplication, rules engine |
| `export` | hledger journal, CSV, JSON output generation |
| `market` | Quote data processing, caching, price history storage (NOT HTTP fetching) |
| `loans` | Amortization, payoff projections, what-if calculations |
| `budget` | Budget engine, allocation, recurring transaction detection |
| `goals` | Goal tracking, timeline projections |

#### Platform-Specific Code

| Platform | Responsibility |
|----------|---------------|
| **CLI** (Rust) | clap commands, ratatui TUI, HTTP via reqwest (acceptable here — no platform API available) |
| **iOS/iPadOS** (Swift) | SwiftUI views, URLSession for HTTP, Keychain/Secure Enclave, Face ID, Widgets, Shortcuts, CloudKit backup |
| **watchOS** (Swift) | SwiftUI complications/views, WatchConnectivity for iPhone companion data |
| **Web** (TypeScript) | Next.js shell, fetch API for HTTP, WebCrypto for key storage, service worker, IndexedDB, D3/visx charts |

#### Cross-Platform Exposure

| Target | Mechanism | Notes |
|--------|-----------|-------|
| CLI | Native Rust binary | Full core, no FFI overhead |
| iOS/iPadOS | UniFFI → XCFramework → Swift Package | Swift async wrapper layer for long-running ops |
| watchOS | UniFFI → minimal subset | Decrypt + read-only queries only |
| Web | wasm-bindgen → npm package | Feature-flagged for bundle size control |

#### WASM Bundle Strategy

Hard budget: **2 MB compressed** for initial load.

```toml
# Cargo.toml feature flags for WASM
[features]
default = ["core"]
core = ["crypto", "accounting", "storage"]     # ~1.5 MB — always loaded
sync = ["core"]                                 # +200 KB — lazy loaded
import-export = ["core"]                        # +300 KB — lazy loaded
market = ["core"]                               # +100 KB — lazy loaded
full = ["core", "sync", "import-export", "market", "loans", "budget", "goals"]
```

- `core` feature ships in initial bundle (vault + accounting + reports)
- Other features lazy-loaded as WASM modules when user accesses them
- Measured in CI: fail build if `core` WASM exceeds 2 MB compressed

#### UniFFI Async Pattern

UniFFI's async support requires a Swift wrapper for idiomatic Swift concurrency:

```swift
// UniFFI generates callback-based API:
func syncVault(callback: @escaping (Result<SyncResult, LdgrError>) -> Void)

// Swift wrapper provides async/await:
public func sync() async throws -> SyncResult {
    try await withCheckedThrowingContinuation { continuation in
        syncVault { result in continuation.resume(with: result) }
    }
}
```

All long-running Rust operations (sync, import, report generation) get this wrapper treatment in the Swift package.

#### Market Data: Platform-Native Fetch, Rust Processing

```
Swift/TS: fetch(url) → raw bytes
    ↓
Rust core: parse(bytes) → QuoteData → store in vault
    ↓
Swift/TS: render(QuoteData) → platform-native charts
```

This avoids pulling reqwest into WASM (large dependency) and uses platform networking (URLSession caching, background fetch, system proxy, energy budgeting).

**Consequences**:
- Rust core is a pure computation library — no I/O, no networking, no platform dependencies
- WASM bundle stays small via feature flags and lazy loading
- Platform-native networking gets caching, proxy, and energy management for free
- UniFFI wrapper adds a thin Swift layer but provides idiomatic APIs

---

### ADR-006: Licensing — Apache-2.0 with AGPL Server + DCO

**Status**: Accepted

**Context**: Need to balance maximum adoption, contributor friendliness, App Store compatibility, and SaaS fork prevention.

**Decision**:

| Component | License | Rationale |
|-----------|---------|-----------|
| Rust core library | Apache-2.0 | Rust ecosystem norm, patent grant, App Store compatible, max adoption |
| CLI | Apache-2.0 | Same as core for simplicity |
| iOS/iPadOS/watchOS app | Apache-2.0 | GPL/AGPL incompatible with App Store TOS |
| Web app | Apache-2.0 | Reduces contributor friction vs AGPL |
| Sync server | AGPL-3.0 | Prevents closed-source SaaS forks of the sync server specifically |

**Additional protections**:
- **DCO (Developer Certificate of Origin)**: All contributions require `Signed-off-by` in commits (like Linux kernel). Lightweight, no CLA signing ceremony.
- **Trademark**: Register "ldgr" as a trademark. Forks must use a different name. This prevents brand confusion even with permissive licensing.

**Why not unified Apache-2.0 everywhere** (per critique suggestion): The sync server is the one component where SaaS exploitation is a real concern. A hosted ldgr-sync service that doesn't share source would undermine the project. AGPL on this single component is targeted and justified. Everything else is permissive.

**Why Apache-2.0 over MIT**: Apache-2.0 includes an explicit patent grant, which matters for cryptographic code. The practical difference is small, but Apache-2.0 is the safer choice for a crypto-heavy project.

**Consequences**:
- Contributors face one license for 90% of the codebase (Apache-2.0)
- AGPL is isolated to the sync server — contributors who avoid AGPL can still contribute to everything else
- Trademark protects the brand regardless of licensing
- DCO ensures contribution rights without CLA overhead

---

## 3. System Architecture

### Component Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                       ldgr Ecosystem                        │
│                                                             │
│  ┌──────────┐  ┌──────────┐  ┌─────────┐  ┌────────────┐  │
│  │  CLI     │  │ iOS/iPad │  │  Watch  │  │   Web App  │  │
│  │ (Rust)   │  │ (Swift)  │  │ (Swift) │  │ (Next.js)  │  │
│  │          │  │          │  │         │  │            │  │
│  │ clap     │  │ SwiftUI  │  │ SwiftUI │  │ React/TS   │  │
│  │ ratatui  │  │ URLSess. │  │ WatchCo │  │ fetch API  │  │
│  │ reqwest  │  │ Keychain │  │ nnect   │  │ IndexedDB  │  │
│  └────┬─────┘  └────┬─────┘  └────┬────┘  └─────┬──────┘  │
│       │              │             │              │         │
│       │         UniFFI bindings    │         WASM bridge    │
│       │              │             │              │         │
│  ┌────▼──────────────▼─────────────▼──────────────▼──────┐  │
│  │                  ldgr-core (Rust)                      │  │
│  │                                                        │  │
│  │  ┌─────────┐ ┌────────────┐ ┌──────────┐ ┌────────┐  │  │
│  │  │ crypto  │ │ accounting │ │ storage  │ │  sync  │  │  │
│  │  │         │ │            │ │          │ │        │  │  │
│  │  │ Argon2  │ │ parser     │ │ SQLite   │ │ events │  │  │
│  │  │ AES-GCM │ │ reports    │ │ versioned│ │ merge  │  │  │
│  │  │ X25519  │ │ queries    │ │ history  │ │ snap-  │  │  │
│  │  │ SRP-6a  │ │ validation │ │          │ │ shots  │  │  │
│  │  └─────────┘ └────────────┘ └──────────┘ └────────┘  │  │
│  │                                                        │  │
│  │  ┌────────┐ ┌────────┐ ┌───────┐ ┌───────┐ ┌───────┐ │  │
│  │  │ import │ │ export │ │ market│ │ loans │ │budget │ │  │
│  │  │ CSV    │ │ hledger│ │ quotes│ │ amort.│ │goals  │ │  │
│  │  │ OFX    │ │ CSV    │ │ cache │ │ payoff│ │project│ │  │
│  │  │ hledger│ │ JSON   │ │ prices│ │ refi  │ │alloc. │ │  │
│  │  └────────┘ └────────┘ └───────┘ └───────┘ └───────┘ │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐   │
│  │           Sync Layer (transport-agnostic)             │   │
│  │                                                       │   │
│  │  ┌──────────┐  ┌──────────┐  ┌───────────────────┐   │   │
│  │  │ Dropbox  │  │ WebDAV / │  │  ldgr-server ✅   │   │   │
│  │  │ API      │  │ S3       │  │  (AGPL-3.0)       │   │   │
│  │  │          │  │          │  │  Encrypted blob    │   │   │
│  │  │          │  │          │  │  store + SRP auth  │   │   │
│  │  └──────────┘  └──────────┘  └───────────────────┘   │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### Data Flow: Transaction Lifecycle

```
User creates transaction
    → ldgr-core validates double-entry balance
    → Encrypt transaction with item key (AES-256-GCM)
    → Write to local SQLite (canonical)
    → Append sync event to outbox
    → [On sync] Batch-encrypt outbox → push to blob store/server
    → [Other device] Pull batches → decrypt → apply to local SQLite
    → [If conflict] Flag for user review in sync_conflicts table
```

---

## 4. Zero-Knowledge Architecture

### 4.1 Key Hierarchy

```
Master Password (user-memorized, never transmitted)
    │
    ▼
Argon2id(password, salt, params)
    │
    ▼
Master Key (MK) ── 256-bit
    │
    ├─── HKDF-SHA256(MK, "ldgr-auth-v1")
    │        ▼
    │    Auth Key ── for SRP-6a server authentication
    │
    └─── HKDF-SHA256(MK, "ldgr-enc-v1")
             ▼
         Master Encryption Key (MEK) ── 256-bit
             │
             └─── AES-256-GCM-Wrap(MEK, VK)
                      ▼
                  Vault Key (VK) ── 256-bit, random
                      │
                      └─── AES-256-GCM-Wrap(VK, IK_n)
                               ▼
                           Item Key (IK) ── per-item, random
```

### 4.2 Argon2id Parameters

| Parameter | Desktop Default | Mobile Default | Notes |
|-----------|----------------|----------------|-------|
| Memory | 256 MB | 64 MB | Tuned per platform |
| Iterations | 3 | 4 | Higher iterations compensate for less memory on mobile |
| Parallelism | 4 | 2 | Match available cores |
| Output | 32 bytes | 32 bytes | 256-bit key |

Parameters are stored in the vault header (unencrypted) and can be upgraded:
- On password change, re-derive with stronger params
- Version field in header signals which params to use

### 4.3 Vault File Format

```
┌──────────────────────────────────────┐
│ Magic bytes: "LDGR" (4 bytes)        │
│ Format version: u16                  │
│ Argon2 params: salt, memory, iter, p │
│ KDF version: u8                      │
│ Encrypted vault metadata blob        │
│   (name, created_at, settings)       │
├──────────────────────────────────────┤
│ Encrypted item 1                     │
│   Item key (wrapped by VK)           │
│   Padded, encrypted payload          │
│   (AES-256-GCM, random IV)          │
├──────────────────────────────────────┤
│ Encrypted item 2                     │
│   ...                                │
├──────────────────────────────────────┤
│ ... more items ...                   │
└──────────────────────────────────────┘
```

**Size-bucket padding**: All encrypted payloads are padded to the nearest bucket before encryption:
- ≤ 512 B → pad to 512 B
- ≤ 2 KB → pad to 2 KB
- ≤ 8 KB → pad to 8 KB
- ≤ 32 KB → pad to 32 KB
- \> 32 KB → pad to nearest 32 KB multiple

This mitigates metadata leakage (observer can't determine transaction complexity from blob size).

**Domain separation**: Each wrapping operation uses distinct AAD (Additional Authenticated Data):
- `"ldgr-vault-wrap-v1"` for VK wrapping
- `"ldgr-item-wrap-v1"` for IK wrapping
- `"ldgr-recovery-wrap-v1"` for recovery key wrapping

### 4.4 Recovery & Key Management

| Scenario | Mechanism |
|----------|-----------|
| **Vault creation** | Generate recovery key (256-bit random), display as printable emergency kit (BIP39-style word list or base32), wrap VK with recovery key |
| **Normal unlock** | Password → Argon2id → MK → MEK → unwrap VK |
| **Password change** | Re-derive MK/MEK from new password, re-wrap VK with new MEK. Item keys are NOT re-encrypted. |
| **Recovery** | Recovery key → unwrap VK directly. User sets new password. |
| **Password + recovery key both lost** | **Data is unrecoverable.** Clear UX: warning at setup, emergency kit download prompt. |
| **Biometric unlock** | MEK cached in platform keychain (iOS Secure Enclave, OS keyring, WebCrypto). Protected by biometrics. Not derived from password on every unlock. |
| **Session timeout** | MEK evicted from memory after configurable timeout (default: 15 min idle). Vault locks. |
| **Key rotation** | VK rotation: generate new VK, re-wrap all IKs. Triggered by: password compromise suspected, admin action. IK rotation: per-item, triggered by item modification (optional). |

### 4.5 Threat Model

| Threat | Protection | Level |
|--------|-----------|-------|
| Server/storage compromise | Server never sees plaintext; all blobs are AES-256-GCM encrypted client-side | ✅ Full |
| Network interception | TLS transport + client-side encryption (defense in depth) | ✅ Full |
| Blob store provider reads data | All data encrypted before upload | ✅ Full |
| Metadata analysis (blob sizes) | Size-bucket padding | ⚠️ Partial (timing, count still visible) |
| Client device theft (locked) | Keychain/Secure Enclave, biometric gate, auto-lock timeout | ⚠️ Partial |
| Client device compromise (root) | Zeroize/ZeroizeOnDrop for keys in memory. OS-level access defeats all protection. | ⚠️ Limited |
| Memory forensics | Keys implement Zeroize; Debug trait redacts secrets | ⚠️ Best-effort |
| Rubber-hose cryptanalysis | Out of scope | ❌ None |

---

## 5. Feature Architecture

### 5.1 Accounting Engine

**Parser**: nom-based (Rust parser combinator), targeting hledger journal format strict subset.

**Core types**:
```rust
struct Transaction {
    id: Uuid,
    date: NaiveDate,
    status: Status,        // Unmarked, Pending, Cleared
    code: Option<String>,
    description: String,
    postings: Vec<Posting>,
    tags: HashMap<String, String>,
    comment: Option<String>,
}

struct Posting {
    account: AccountName,  // colon-separated hierarchy
    amount: Option<Amount>,
    balance_assertion: Option<Amount>,
    status: Status,
    comment: Option<String>,
    tags: HashMap<String, String>,
}

struct Amount {
    quantity: Decimal,     // rust_decimal for precision
    commodity: Commodity,
}
```

**Reports**:

| Report | Description |
|--------|-------------|
| Balance | Account balances at a point in time, hierarchical |
| Register | Chronological transaction list with running balance |
| Income Statement | Revenue - Expenses for a period |
| Balance Sheet | Assets - Liabilities = Equity at a point in time |
| Cash Flow | Cash inflows/outflows by category |
| Trial Balance | All accounts with debit/credit totals |

**Query language**: Subset of hledger's query syntax:
- `acct:Expenses` — filter by account
- `desc:grocery` — filter by description
- `date:2024` — filter by date/period
- `amt:>100` — filter by amount
- `tag:project=alpha` — filter by tag
- Boolean: `AND`, `OR`, `NOT`

### 5.2 Import & Reconciliation

**Import pipeline**:
```
File (CSV/OFX/QFX/hledger)
    → Format detection (magic bytes / extension)
    → Parser (format-specific)
    → Normalization (→ canonical Transaction struct)
    → Deduplication (fuzzy match: date ±2 days, amount exact, payee similarity >0.8)
    → Rules engine (payee → account mapping)
    → User review (confirm/edit/skip)
    → Write to vault
```

**CSV import**: Configurable column mapping stored as reusable profiles:
```toml
[csv-profile.chase-checking]
date_column = 0
date_format = "%m/%d/%Y"
description_column = 2
amount_column = 3
skip_header = true
default_account = "Assets:Checking:Chase"
```

**Rules engine**: Pattern-based auto-categorization:
```toml
[[rule]]
payee_contains = "WHOLE FOODS"
account = "Expenses:Food:Groceries"

[[rule]]
payee_regex = "AMZN|AMAZON"
account = "Expenses:Shopping:Online"
```

### 5.3 Investment Portfolio

**Lot tracking**:
```rust
struct Lot {
    id: Uuid,
    commodity: Commodity,
    quantity: Decimal,
    cost_basis: Amount,        // total cost
    cost_per_unit: Amount,
    acquisition_date: NaiveDate,
    account: AccountName,
}
```

**Cost basis methods**: FIFO, LIFO, Specific Identification, Average Cost — selectable per account or per disposal.

**Performance metrics**:
- Unrealized gain/loss: current market value - cost basis
- Realized gain/loss: proceeds - cost basis (on sell/dispose)
- Time-weighted return (TWR): eliminates cash flow timing effects
- Money-weighted return (MWR/IRR): includes cash flow timing
- Short-term vs long-term classification (configurable holding period, default 1 year)

**Dividend tracking**: DRIP support (auto-create new lots from reinvested dividends).

### 5.4 Market Data

**Provider trait** (I/O-free — builds URLs and parses responses):

```rust
trait QuoteProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn supported_asset_classes(&self) -> Vec<AssetClass>;
    fn quote_url(&self, symbols: &[&str]) -> String;
    fn parse_quotes(&self, response: &str) -> Result<Vec<Quote>, MarketError>;
    fn historical_url(&self, symbol: &str, range: &DateRange) -> String;
    fn parse_historical(&self, response: &str) -> Result<Vec<Ohlcv>, MarketError>;
    fn metadata(&self) -> ProviderMetadata; // default provided
}
```

**Built-in providers**:

| Provider | ID | Asset Classes | API Key | Rate Limit |
| --- | --- | --- | --- | --- |
| Yahoo Finance | `yahoo-finance` | Stocks, ETFs, Mutual Funds, Indices, Forex, Crypto | No (TOS caveat) | ~2000/hr |
| CoinGecko | `coingecko` | Crypto | No (free tier) | 5–15/min |
| ECB | `ecb` | Forex (EUR base) | No | Daily update |

**Provider registry**: `ProviderRegistry` provides discovery and lookup by ID or asset class. Community providers register via `registry.register(Box::new(MyProvider))`. See `docs/provider-development-guide.md` and `examples/ldgr-provider-example/`.

**Provider chain**: `ProviderChain` routes requests by asset class with fallback on failure. Both use `builtin_providers()` as a single source of truth for the default set.

**Caching**: SQLite-backed cache with configurable TTL (default: 15 min for intraday, 24 hr for daily).

**CLI TUI** (ratatui):
- Watchlist view: symbol, price, change, change %, sparkline
- Portfolio view: holdings with market value, gain/loss, allocation %
- Chart view: interactive line/candlestick charts with zoom (1D, 5D, 1M, 3M, 1Y, 5Y)
- Standalone mode: `ldgr watch AAPL MSFT GOOG BTC-USD` works without a vault

### 5.5 Loan Tracking

**Loan model**:
```rust
struct Loan {
    id: Uuid,
    name: String,
    loan_type: LoanType,         // Mortgage, Auto, Student, Personal, HELOC
    principal: Decimal,
    interest_rate: Decimal,       // annual rate
    rate_type: RateType,          // Fixed, Variable { adjusts_every, index, margin }
    term_months: u32,
    start_date: NaiveDate,
    payment_amount: Decimal,
    extra_payment: Decimal,
    linked_account: AccountName,  // liability account in ledger
}
```

**Amortization**: Generate month-by-month schedule (principal, interest, balance).

**What-if analysis**:
- Extra payment scenarios: "pay $200/mo extra → save $X in interest, pay off Y months early"
- Biweekly payments: half-payment every 2 weeks = 13 payments/year
- Refinance comparison: new rate + closing costs vs remaining term

**Accounting integration**: When a loan payment is imported, automatically split into:
- `Liabilities:Mortgage:Principal` (debit)
- `Expenses:Interest:Mortgage` (debit)
- `Assets:Checking` (credit)

### 5.6 Budgeting

**Methods**:
- **Envelope/category**: Allocate fixed amounts to expense categories
- **Zero-based**: Every dollar of income is assigned a job
- **Percentage-based**: 50/30/20 (needs/wants/savings) or custom ratios
- **Custom**: User-defined rules

**Recurring transaction detection** (rule-based):
- Group transactions by normalized payee + approximate amount
- Detect frequency (weekly, biweekly, monthly, quarterly, annual)
- Present detected patterns to user for confirmation
- Flagging when expected recurring transaction is missing

### 5.7 Financial Goals

**Goal types**: Savings target, debt payoff, investment milestone, emergency fund, retirement, custom.

**Projections**:
- Linear: "At $X/month, reach target by DATE"
- Compound: "At Y% return, reach target by DATE"
- What-if: "If you increase by $Z/month, reach target N months sooner"

**Integration**: Goals can be linked to budget categories (auto-allocate) and accounts (track balance toward target).

### 5.8 Net Worth

**Aggregation**:
- Sum all asset accounts (checking, savings, investments at market value, real estate at manual valuation)
- Subtract all liability accounts (credit cards, loans at current balance)
- Historical tracking: snapshot net worth daily/weekly (configurable)

**Breakdown views**: By asset class, by account type, liquid vs illiquid, by institution.

### 5.9 Reports & Export

**Export formats**: CSV, JSON, hledger journal (one-way), PDF (via headless rendering or CLI table formatting).

**Unencrypted vault export**: `ldgr export --full --unencrypted` dumps all data as structured JSON. User's right to their own data.

**CLI output modes**: `--output table|json|csv` on all commands for scripting and piping.

### 5.10 Sync Server (`ldgr-server`)

The self-hosted sync server is an encrypted blob relay built with Axum. It never
decrypts user data — clients authenticate via SRP-6a zero-knowledge proofs and
all synced data is stored as opaque encrypted blobs.

**Authentication**: SRP-6a (RFC 5054, 2048-bit group, SHA-256). The server stores
only salt + verifier; passwords never leave the client. Login is a two-step
handshake (init → verify) that produces a session token. Session tokens are
SHA-256 hashed before storage — the raw token is returned to the client once and
never stored.

**API** (17 endpoints under `/api/v1/`):

| Group | Endpoints | Purpose |
|-------|-----------|---------|
| Auth | `POST register`, `POST login/init`, `POST login/verify`, `POST logout` | SRP-6a account creation and session management |
| Vaults | `POST /vaults`, `GET /vaults` | Create and list vaults scoped to authenticated user |
| Batches | `PUT`, `GET`, `GET list` under `/vaults/:id/batches/` | Encrypted sync batch blob CRUD with `?since`, `?device_id`, `?limit` filters |
| Snapshots | `PUT`, `GET`, `GET list` under `/vaults/:id/snapshots/` | Encrypted snapshot blob CRUD for new-device onboarding |
| Devices | `GET list`, `PUT`, `DELETE` under `/vaults/:id/devices/` | Device registration and removal (encrypted device info) |
| Relay | `POST offer`, `GET offer`, `POST respond`, `GET response` | Ephemeral key exchange relay for device onboarding |
| Health | `GET /health` | Server health check |

**Storage**: Server-side SQLite with WAL mode, foreign keys, and 5s busy timeout.
All DB access goes through `tokio::task::spawn_blocking` to avoid blocking the
async runtime. Six tables: `users`, `sessions`, `vaults`, `blobs`, `devices`,
`relay_offers`.

**Key design decisions**:
- Blob writes use put-if-absent semantics (409 Conflict on duplicate path)
- Content hashes (SHA-256) stored alongside blobs for integrity verification
- Vault access is ownership-scoped: returns 404 (not 403) to avoid leaking vault existence
- SRP handshake state is in-memory (HashMap with TTL + cap at 100 pending)
- Relay offers are ephemeral with configurable TTL (default 10 minutes)
- Body size limits: 64 KB for JSON endpoints, configurable for blob endpoints (default 50 MB)

**Deployment**: Docker multi-stage build (`rust:1.88-bookworm` → `debian:bookworm-slim`)
with non-root user and `/data` volume for the SQLite database.

---

## 6. Data Model

### SQLite Schema (Core)

```sql
-- Accounts
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,           -- UUID
    name TEXT NOT NULL UNIQUE,     -- "Assets:Checking:Chase"
    type TEXT NOT NULL,            -- Asset, Liability, Income, Expense, Equity
    commodity TEXT,                -- default commodity for this account
    parent_id TEXT REFERENCES accounts(id),
    note TEXT,
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0
);

-- Transactions
CREATE TABLE transactions (
    id TEXT PRIMARY KEY,           -- UUID
    date TEXT NOT NULL,            -- ISO 8601 date
    status TEXT NOT NULL DEFAULT 'unmarked', -- unmarked, pending, cleared
    code TEXT,
    description TEXT NOT NULL,
    comment TEXT,
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0
);

-- Postings (belong to transactions, enforce double-entry)
CREATE TABLE postings (
    id TEXT PRIMARY KEY,
    transaction_id TEXT NOT NULL REFERENCES transactions(id),
    account_id TEXT NOT NULL REFERENCES accounts(id),
    amount_quantity TEXT,          -- Decimal as string for precision
    amount_commodity TEXT,
    balance_assertion_quantity TEXT,
    balance_assertion_commodity TEXT,
    posting_order INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1
);

-- Tags (on transactions and postings)
CREATE TABLE tags (
    id TEXT PRIMARY KEY,
    entity_type TEXT NOT NULL,     -- transaction, posting
    entity_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT
);

-- Commodities
CREATE TABLE commodities (
    symbol TEXT PRIMARY KEY,
    name TEXT,
    decimal_places INTEGER DEFAULT 2,
    format TEXT                    -- display format
);

-- Market Prices
CREATE TABLE prices (
    id TEXT PRIMARY KEY,
    commodity TEXT NOT NULL REFERENCES commodities(symbol),
    currency TEXT NOT NULL,
    price TEXT NOT NULL,           -- Decimal as string
    date TEXT NOT NULL,
    source TEXT,                   -- manual, yahoo, coingecko, etc.
    created_at TEXT NOT NULL
);

CREATE INDEX idx_prices_commodity_date ON prices(commodity, date DESC);

-- Investment Lots
CREATE TABLE lots (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    commodity TEXT NOT NULL REFERENCES commodities(symbol),
    quantity TEXT NOT NULL,
    cost_basis TEXT NOT NULL,
    cost_per_unit TEXT NOT NULL,
    cost_commodity TEXT NOT NULL,
    acquisition_date TEXT NOT NULL,
    disposal_date TEXT,
    disposal_proceeds TEXT,
    realized_gain TEXT,
    created_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0
);

-- Loans
CREATE TABLE loans (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    loan_type TEXT NOT NULL,       -- mortgage, auto, student, personal, heloc
    principal TEXT NOT NULL,
    interest_rate TEXT NOT NULL,
    rate_type TEXT NOT NULL,       -- fixed, variable
    term_months INTEGER NOT NULL,
    start_date TEXT NOT NULL,
    payment_amount TEXT NOT NULL,
    extra_payment TEXT DEFAULT '0',
    linked_account_id TEXT REFERENCES accounts(id),
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0
);

-- Budgets
CREATE TABLE budgets (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    method TEXT NOT NULL,          -- envelope, zero_based, percentage, custom
    period TEXT NOT NULL,          -- monthly, weekly, custom
    start_date TEXT NOT NULL,
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE budget_allocations (
    id TEXT PRIMARY KEY,
    budget_id TEXT NOT NULL REFERENCES budgets(id),
    account_id TEXT NOT NULL REFERENCES accounts(id),
    amount TEXT NOT NULL,
    rollover INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1
);

-- Goals
CREATE TABLE goals (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    goal_type TEXT NOT NULL,       -- savings, debt_payoff, investment, emergency, retirement
    target_amount TEXT NOT NULL,
    target_date TEXT,
    linked_account_id TEXT REFERENCES accounts(id),
    linked_budget_id TEXT REFERENCES budgets(id),
    created_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0
);

-- Import Rules
CREATE TABLE import_rules (
    id TEXT PRIMARY KEY,
    priority INTEGER NOT NULL DEFAULT 0,
    payee_pattern TEXT NOT NULL,    -- exact, contains, or regex
    match_type TEXT NOT NULL,       -- exact, contains, regex
    target_account_id TEXT NOT NULL REFERENCES accounts(id),
    created_at TEXT NOT NULL
);

-- Sync
CREATE TABLE sync_outbox (
    id TEXT PRIMARY KEY,           -- UUIDv7
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    operation TEXT NOT NULL,        -- create, update, delete
    payload BLOB NOT NULL,         -- serialized entity state (pre-encryption)
    created_at TEXT NOT NULL,
    synced INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE sync_state (
    device_id TEXT PRIMARY KEY,
    vector_clock TEXT NOT NULL,     -- JSON: { "device_a": 42, "device_b": 37 }
    last_sync_at TEXT
);

CREATE TABLE sync_conflicts (
    id TEXT PRIMARY KEY,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    local_version BLOB NOT NULL,
    remote_version BLOB NOT NULL,
    detected_at TEXT NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0,
    resolution TEXT                 -- 'local', 'remote', 'merged'
);

-- Net Worth Snapshots
CREATE TABLE networth_snapshots (
    id TEXT PRIMARY KEY,
    date TEXT NOT NULL,
    total TEXT NOT NULL,
    breakdown TEXT NOT NULL,        -- JSON: { "liquid": "X", "investments": "Y", ... }
    created_at TEXT NOT NULL
);
```

### SQLite Schema (Sync Server)

The sync server uses its own separate SQLite database. This schema stores
encrypted blobs and authentication data — no plaintext financial data.

```sql
CREATE TABLE users (
    id          TEXT PRIMARY KEY,
    username    TEXT UNIQUE NOT NULL,
    salt        BLOB NOT NULL,         -- SRP-6a salt
    verifier    BLOB NOT NULL,         -- SRP-6a verifier (never stores password)
    created_at  TEXT NOT NULL
);

CREATE TABLE sessions (
    token_hash  TEXT PRIMARY KEY,      -- SHA-256(raw_token); raw token never stored
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL,
    expires_at  TEXT NOT NULL
);

CREATE TABLE vaults (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL
);

CREATE TABLE blobs (
    path          TEXT PRIMARY KEY,    -- e.g. "{vault_id}/batches/{device_id}/{batch_id}.enc"
    vault_id      TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    data          BLOB NOT NULL,       -- opaque encrypted content
    size          INTEGER NOT NULL,
    content_hash  TEXT NOT NULL,       -- SHA-256 for integrity verification
    created_at    TEXT NOT NULL
);

CREATE TABLE devices (
    id              TEXT NOT NULL,
    vault_id        TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
    encrypted_info  BLOB NOT NULL,     -- opaque encrypted device info
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (vault_id, id)
);

CREATE TABLE relay_offers (
    id              TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    offer_data      BLOB NOT NULL,     -- encrypted key exchange offer
    response_data   BLOB,              -- encrypted key exchange response
    created_at      TEXT NOT NULL,
    expires_at      TEXT NOT NULL       -- ephemeral, default 10 min TTL
);
```

---

## 7. Platform Design

### 7.1 CLI Commands

```
ldgr — Zero-knowledge bookkeeping

VAULT
  ldgr init                    Create a new vault
  ldgr unlock                  Unlock vault (password prompt)
  ldgr lock                    Lock vault (clear session)
  ldgr status                  Show vault status (locked/unlocked, device, sync)
  ldgr backup [path]           Export encrypted vault file
  ldgr restore <path>          Restore vault from backup
  ldgr change-password         Change master password
  ldgr recovery-kit            Display/regenerate recovery key

ACCOUNTING
  ldgr add                     Add a transaction (interactive)
  ldgr edit <id>               Edit a transaction
  ldgr delete <id>             Delete a transaction
  ldgr accounts [query]        List accounts
  ldgr balance [query]         Balance report
  ldgr register [query]        Register report (transaction list)
  ldgr incomestatement [query] Income statement
  ldgr balancesheet [query]    Balance sheet
  ldgr cashflow [query]        Cash flow statement

IMPORT / EXPORT
  ldgr import <file>           Import CSV/OFX/QFX/hledger journal
  ldgr validate <file>         Check if journal is importable
  ldgr export [--format F]     Export data (hledger, csv, json)
  ldgr rules                   Manage auto-categorization rules
  ldgr reconcile <account>     Reconciliation workflow

INVESTMENTS
  ldgr portfolio               Portfolio summary (holdings, value, allocation)
  ldgr lots [account]          View tax lots
  ldgr gains [--realized]      Capital gains report
  ldgr performance [account]   TWR/MWR performance metrics

MARKET
  ldgr watch [symbols...]      Live market watchlist (TUI)
  ldgr quote <symbol>          Get current quote
  ldgr chart <symbol>          Interactive price chart (TUI)
  ldgr prices update           Update all tracked commodity prices

LOANS
  ldgr loans                   List tracked loans
  ldgr amortize <loan>         Show amortization schedule
  ldgr payoff <loan>           Payoff projections
  ldgr refinance <loan>        Refinance what-if analysis

BUDGET & GOALS
  ldgr budget                  Budget overview (allocated vs actual)
  ldgr budget set              Set/edit budget allocations
  ldgr goals                   Goal progress summary
  ldgr goals add               Add a financial goal
  ldgr networth                Net worth summary with history

SYNC
  ldgr sync                    Sync vault across devices
  ldgr sync setup              Configure sync transport
  ldgr sync status             Show sync status and conflicts
  ldgr sync resolve            Resolve sync conflicts
  ldgr devices                 List linked devices
  ldgr devices add             Link a new device (QR code)

CONFIG
  ldgr config                  View/edit configuration
  ldgr config providers        Manage market data provider API keys

GLOBAL FLAGS
  --output table|json|csv      Output format (default: table)
  --no-color                   Disable color output
  --vault <path>               Vault file path (default: ~/.ldgr/vault.ldgr)
  --verbose                    Verbose output
```

### 7.2 iOS/iPad App Structure

**Navigation**: Tab bar (iPhone) / Sidebar (iPad)
- **Dashboard**: Net worth, recent transactions, budget summary, goal progress
- **Transactions**: List, search, filter, add/edit
- **Accounts**: Account list with balances, drill into register
- **Investments**: Portfolio, lots, performance charts
- **Budget**: Envelope view, actuals, spending trends
- **Market**: Watchlist, charts, quotes
- **More**: Loans, goals, reports, settings, sync

**iPad enhancements**: Split view (account list + register), keyboard shortcuts, drag-and-drop CSV import.

### 7.3 Apple Watch

**Read-only glances** (implemented):

- Net worth glance with multi-currency support
- Portfolio holdings summary grouped by commodity
- Monthly spending breakdown with top 5 expense categories
- Market watchlist (pending market data feature)

**Complications** (WidgetKit):

- **Net Worth** — rectangular, circular, inline families
- **Daily Spend** — circular and inline families
- **Portfolio** — rectangular and circular families
- Hourly timeline refresh with immediate reload on data update

**Data flow**:

```
VaultDataStore → WatchConnectivityManager → WCSession.updateApplicationContext
    → PhoneConnectivityManager → UserDefaults (App Group) → WidgetKit TimelineProvider
```

**Data source**: Pre-computed `WatchSummary` pushed from iPhone companion app via WatchConnectivity. Watch does NOT decrypt the full vault. Summary includes net worth, portfolio, budget, daily spend, and watchlist entries.

**Project structure**:

- `apps/ios/LdgrShared/Sources/` — `WatchSummary` model (compiled into iOS, watchOS, and widget targets)
- `apps/ios/ldgrWatch/Sources/` — Watch app (SwiftUI views, WCSession delegate)
- `apps/ios/ldgrWatch/Widgets/` — WidgetKit complication extension
- App Group `group.com.kafkade.ldgr.watch` shared between watch app and widget extension

### 7.4 iOS Widgets & Siri Shortcuts

**Home screen widgets** (WidgetKit):

- **Net Worth** — small (primary currency) and medium (multi-currency) families
- **Monthly Spending** — medium; shows month label, total, top 3 categories, today's spend
- **Portfolio** — medium; shows holdings by commodity
- 30-minute timeline refresh (best-effort) with immediate reload on vault data changes
- All widgets display "Unlock ldgr to view" when the vault is locked

**Siri Shortcuts** (App Intents):

- **Query Net Worth** — returns cached net worth via dialog, no app launch needed
- **Check Monthly Spending** — returns month total and top categories via dialog
- **Add Expense** — opens app with pre-filled parameters (amount as String, not Double, for decimal precision)

**Privacy**:

- Widget data is pre-computed and written to App Group `group.com.kafkade.ldgr`
- `WidgetDataManager` clears cached data and reloads all timelines when the vault locks
- Intents that read cached data return "Please unlock ldgr first" when cache is cleared
- `AddExpenseIntent` uses `openAppWhenRun = true` to require interactive vault unlock

**Data flow**:

```
VaultDataStore → WidgetDataManager → UserDefaults (App Group) → WidgetKit TimelineProvider
                                   → App Intents (read-only)
Lock event   → WidgetDataManager.clearOnLock() → removes cache → reloads timelines
```

**Project structure**:

- `apps/ios/ldgr/Sources/Services/WidgetDataManager.swift` — computes and caches summaries
- `apps/ios/ldgr/Sources/Intents/` — App Intent definitions and shortcuts provider
- `apps/ios/ldgrWidgets/Sources/` — Widget extension (bundle, providers, views)
- App Group `group.com.kafkade.ldgr` shared between iOS app and widget extension

### 7.4 Web App

**Architecture**: Next.js with app router. Static shell + client-side WASM for all vault operations.

**Pages**:
- `/` — Marketing/landing page (SSR)
- `/app` — Dashboard (client-side, WASM)
- `/app/transactions` — Transaction list
- `/app/accounts` — Account management
- `/app/investments` — Portfolio view
- `/app/budget` — Budget management
- `/app/market` — Market watchlist
- `/app/reports` — Report generation
- `/app/settings` — Vault settings, sync config

**No SSR for user data**: Server never processes user data. All vault operations happen in the browser via WASM.

---

## 8. Repository Structure

```
ldgr/
├── Cargo.toml                  # Workspace manifest
├── LICENSE                     # Apache-2.0
├── README.md
├── CONTRIBUTING.md
├── DCO                         # Developer Certificate of Origin
├── .github/
│   ├── workflows/
│   │   ├── ci.yml             # Build + test all platforms
│   │   ├── release.yml        # Release binaries + packages
│   │   └── wasm-size.yml      # WASM bundle size check
│   └── ISSUE_TEMPLATE/
│
├── crates/
│   ├── ldgr-core/             # Core library (Apache-2.0)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── crypto/        # Key hierarchy, vault, Argon2id, SRP-6a
│   │       ├── accounting/    # Parser, validation, reports, queries
│   │       ├── storage/       # SQLite schema, CRUD, versioned history
│   │       ├── sync/          # Event generation, merge, conflict detection
│   │       ├── import/        # CSV, OFX, hledger parsing
│   │       ├── export/        # hledger, CSV, JSON output
│   │       ├── market/        # Quote processing, caching, price storage
│   │       ├── loans/         # Amortization, projections
│   │       ├── budget/        # Budget engine, recurring detection
│   │       └── goals/         # Goal tracking, projections
│   │
│   ├── ldgr-cli/              # CLI application (Apache-2.0)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── commands/      # clap command handlers
│   │       ├── tui/           # ratatui market tracker, interactive views
│   │       └── session.rs     # MEK session management
│   │
│   └── ldgr-server/           # Sync server (AGPL-3.0)
│       ├── Cargo.toml
│       ├── LICENSE             # AGPL-3.0 (overrides workspace)
│       ├── Dockerfile          # Multi-stage build (rust → debian-slim)
│       └── src/
│           ├── main.rs         # Axum entry point, tracing setup
│           ├── config.rs       # Env-based configuration
│           ├── error.rs        # ServerError → HTTP status mapping
│           ├── state.rs        # AppState (DB + SRP store + config)
│           ├── api/            # Route handlers
│           │   ├── mod.rs      # Router assembly, body limits
│           │   ├── auth.rs     # Register, login (SRP), logout
│           │   ├── vaults.rs   # Create/list vaults, access checks
│           │   ├── batches.rs  # Encrypted batch blob CRUD
│           │   ├── snapshots.rs# Encrypted snapshot blob CRUD
│           │   ├── devices.rs  # Device registration/removal
│           │   └── relay.rs    # Key exchange offer/response
│           ├── auth/           # Authentication layer
│           │   ├── mod.rs      # Hex encode/decode utilities
│           │   ├── srp.rs      # SRP-6a (RFC 5054, 2048-bit)
│           │   ├── session.rs  # Token generation, SHA-256 hashing
│           │   └── middleware.rs# AuthUser extractor
│           └── storage/        # Server-side SQLite
│               ├── mod.rs      # CRUD ops via spawn_blocking
│               └── schema.rs   # 6 tables (users, sessions, vaults, blobs, devices, relay)
│
├── bindings/
│   └── swift/                 # UniFFI-generated Swift bindings
│       ├── Package.swift
│       └── Sources/
│           ├── LdgrCore/      # Generated UniFFI bindings
│           └── LdgrSwift/     # Idiomatic Swift async wrappers
│
├── apps/
│   ├── ios/                   # iOS/iPadOS/watchOS app (Apache-2.0)
│   │   ├── project.yml        # XcodeGen spec (iOS + watchOS targets)
│   │   ├── LdgrShared/        # Shared types (compiled into iOS + watchOS)
│   │   │   └── Sources/
│   │   │       └── WatchSummary.swift
│   │   ├── ldgr/              # iOS app target
│   │   │   └── Sources/
│   │   │       ├── LdgrApp.swift
│   │   │       ├── Services/  # Keychain, Biometric, Sync, WatchConnectivity, WidgetDataManager
│   │   │       ├── Intents/   # App Intents: QueryNetWorth, CheckBudget, AddExpense, Shortcuts
│   │   │       └── Views/     # Dashboard, Transactions, Accounts, etc.
│   │   ├── ldgrWatch/         # watchOS app target
│   │   │   ├── Sources/
│   │   │   │   ├── LdgrWatchApp.swift
│   │   │   │   ├── PhoneConnectivityManager.swift
│   │   │   │   └── Views/    # WatchHome, NetWorth, Portfolio, Budget, NoData
│   │   │   ├── Widgets/       # WidgetKit complication extension
│   │   │   │   └── LdgrWidgets.swift
│   │   │   └── Resources/
│   │   ├── ldgrWidgets/         # iOS home screen widget extension
│   │   │   └── Sources/
│   │   │       ├── LdgrWidgets.swift
│   │   │       └── Views/    # NetWorthWidget, BudgetWidget, PortfolioWidget
│   └── web/                   # Next.js web app (Apache-2.0)
│       ├── package.json
│       ├── next.config.js
│       ├── src/
│       │   ├── app/           # Next.js app router
│       │   ├── components/    # React components
│       │   ├── lib/           # WASM bridge, WebCrypto
│       │   └── workers/       # Service worker for offline
│       └── wasm/              # WASM build output
│
├── docs/
│   ├── adr/                   # Architecture Decision Records
│   ├── journal-subset.md      # Supported hledger syntax spec
│   ├── vault-format.md        # Vault binary format spec
│   ├── sync-protocol.md       # Sync protocol spec
│   ├── threat-model.md        # Security threat model
│   └── provider-development-guide.md  # Community provider dev guide
│
├── examples/
│   └── ldgr-provider-example/ # Template for community market data providers
│
└── tests/
    ├── fixtures/              # Test journals, CSV files, OFX samples
    ├── conformance/           # hledger conformance tests
    └── integration/           # Cross-crate integration tests
```

---

## 9. Technology Choices

### Rust Core

| Dependency | Purpose | Justification |
|------------|---------|---------------|
| `aes-gcm` | AES-256-GCM encryption | RustCrypto, audited, pure Rust |
| `argon2` | Password hashing | RustCrypto, OWASP recommended |
| `x25519-dalek` | Key exchange | Well-tested, used by Signal |
| `hkdf` | Key derivation | RustCrypto, standard |
| `blake2` | Hashing | Fast, secure, RustCrypto |
| `srp` | SRP-6a authentication | Zero-knowledge auth |
| `rusqlite` | SQLite (desktop) | Mature, well-maintained |
| `sql-js` | SQLite (WASM) | Via wasm-bindgen bridge |
| `nom` or `winnow` | Journal parsing | Fast, composable parser combinators |
| `rust_decimal` | Decimal arithmetic | Exact decimal for financial math |
| `chrono` | Date/time handling | Comprehensive, well-maintained |
| `uuid` | UUIDv7 generation | Time-ordered unique IDs |
| `serde` / `serde_json` | Serialization | De facto standard |
| `zeroize` | Secure memory clearing | Keys cleared on drop |
| `uniffi` | Swift/Kotlin bindings | Mozilla-maintained, production-proven |
| `wasm-bindgen` | WASM bindings | Official Rust-WASM toolchain |

### CLI

| Dependency | Purpose |
|------------|---------|
| `clap` | Command-line argument parsing |
| `ratatui` + `crossterm` | Terminal UI (market tracker, charts) |
| `reqwest` | HTTP client (market data) |
| `rpassword` | Secure password input |
| `comfy-table` | Table output formatting |
| `indicatif` | Progress bars |

### Sync Server

| Dependency | Purpose |
|------------|---------|
| `axum` | Async HTTP framework |
| `tokio` | Async runtime |
| `tower-http` | CORS, tracing middleware |
| `tracing` + `tracing-subscriber` | Structured logging |
| `rusqlite` | Server-side SQLite storage |
| `num-bigint` | SRP-6a big integer arithmetic |
| `sha2` | SHA-256 for SRP proofs and content hashes |
| `rand` | Cryptographic random number generation |
| `chrono` | Timestamps and TTL calculations |

### iOS/iPadOS

| Technology | Purpose |
|------------|---------|
| SwiftUI | UI framework |
| Swift Concurrency | async/await for Rust core calls |
| Keychain Services | MEK storage with biometric protection |
| URLSession | HTTP for market data, sync transport |
| WidgetKit | Home screen widgets |
| App Intents | Siri Shortcuts |

### Web

| Technology | Purpose |
|------------|---------|
| Next.js 14+ | App framework (app router) |
| TypeScript | Type safety |
| D3.js or visx | Charts and visualization |
| IndexedDB | Local vault cache |
| Web Crypto API | Key storage |
| Service Worker | Offline support |

---

## 10. Phased Roadmap

### Phase 0 — Foundation

**Goal**: Buildable monorepo with crypto module and basic vault operations.

**Deliverables**:
- Monorepo structure with CI/CD (GitHub Actions)
- `ldgr-core` crate: crypto module (key hierarchy, Argon2id, AES-256-GCM, vault encrypt/decrypt)
- Vault file format implementation (create, open, lock, unlock)
- Recovery key generation and emergency kit
- SQLite schema (accounts, transactions, postings, commodities)
- `ldgr init`, `ldgr unlock`, `ldgr lock`, `ldgr status` CLI commands
- Property-based tests for crypto module
- README, CONTRIBUTING.md, DCO, LICENSE

**Exit criteria**: Can create a vault, unlock it, lock it, and verify the crypto round-trips correctly.

---

### Phase 1 — CLI Ledger (MVP)

**Goal**: Usable single-device ledger. A user can track their finances entirely via CLI.

**Deliverables**:
- hledger journal parser (strict subset v1.0)
- Account management (`ldgr accounts`)
- Transaction entry and editing (`ldgr add`, `ldgr edit`, `ldgr delete`)
- CSV import with configurable column mapping (`ldgr import`)
- Import rules engine (payee → account auto-categorization)
- Basic reports: `balance`, `register`, `incomestatement`, `balancesheet`
- Multi-currency support with manual price entries
- Query language (account, description, date, amount filters)
- `--output table|json|csv` on all commands
- Journal validation tool (`ldgr validate`)
- Conformance tests against hledger binary output

**Exit criteria**: A user can import their bank CSV, categorize transactions, and generate a balance report — all encrypted at rest.

---

### Phase 2 — Import, Investments & Net Worth

**Goal**: Full import pipeline and investment tracking.

**Deliverables**:
- OFX/QFX import support
- Reconciliation workflow (`ldgr reconcile`)
- Deduplication (fuzzy matching on import)
- Investment tracking: lots, cost basis (FIFO/LIFO/SpecID/AvgCost), gains
- Price directives and history
- Market data: basic quote fetching (Yahoo Finance provider)
- Net worth tracking with historical snapshots
- Enhanced reports: cash flow, trial balance, capital gains
- Export: CSV, JSON, hledger journal (one-way)

**Exit criteria**: A user can track investments with tax lots and see their net worth over time.

---

### Phase 3 — Market Tracker & Budgeting

**Goal**: Rich market data TUI and budgeting module.

**Deliverables**:
- Pluggable market data provider trait + Alpha Vantage, CoinGecko providers
- CLI TUI: real-time watchlist, portfolio view, interactive charts (line, candlestick)
- Standalone market tracker mode (`ldgr watch` works without vault)
- Caching layer with configurable TTL and rate limiting
- Budgeting module: envelope + zero-based methods
- Recurring transaction detection (rule-based)
- Budget vs actual reporting
- Financial goals: basic goal tracking with linear projections
- `ldgr budget`, `ldgr goals`, `ldgr networth` commands

**Exit criteria**: A user can monitor markets in real-time, set budgets, and track financial goals.

---

### Phase 4 — Sync & iPhone/iPad

**Goal**: Cross-device sync and native Apple apps.

**Deliverables**:
- Sync engine: event generation, batch encryption, conflict detection
- Blob store sync transport (Dropbox API, WebDAV, S3-compatible)
- Conflict resolution UI (CLI: interactive merge, iOS: side-by-side diff)
- Snapshot/compaction for efficient new-device onboarding
- Device onboarding via QR code key exchange
- UniFFI bindings → Swift Package
- Swift async wrapper layer
- iOS app: dashboard, transactions, accounts, investments, budget
- iPad: multi-column layouts, sidebar navigation
- Keychain/Secure Enclave integration, Face ID/Touch ID unlock
- Background sync, offline-first with local vault cache
- `ldgr sync`, `ldgr devices` CLI commands

**Exit criteria**: A user can add a transaction on their phone and see it on their laptop after sync, with conflicts handled gracefully.

---

### Phase 5 — Web & Advanced Features

**Goal**: Web app and advanced financial tools.

**Deliverables**:
- WASM build of ldgr-core (feature-flagged, < 2 MB compressed)
- Next.js web app: dashboard, transactions, investments, budget, market
- Client-side-only vault operations (no server rendering of user data)
- Service worker for offline access
- ~~Self-hosted sync server (AGPL-3.0, Axum-based, encrypted blob store)~~ ✅
- ~~SRP-6a authentication for server sync~~ ✅
- Loan tracking module: amortization, payoff projections, what-if, refinance
- Advanced goal projections (compound growth, what-if scenarios)
- PDF report generation

**Exit criteria**: A user can access their finances from any browser with full offline support, and optionally self-host a sync server.

---

### Phase 6 — Polish & Ecosystem

**Goal**: Apple Watch, widgets, and community ecosystem.

**Deliverables**:
- ~~Apple Watch app: net worth glance, portfolio summary, budget remaining~~ ✅
- ~~Watch complications: net worth, daily spend, portfolio gain/loss~~ ✅
- ~~iOS Widgets: net worth, monthly spending, portfolio value~~ ✅
- ~~Siri Shortcuts: query net worth, check spending, add expense~~ ✅
- ~~Community market data provider interface + documentation~~ ✅
- Themes for CLI and web
- Plugin/extension system for advanced features (jurisdiction-specific tax rules, etc.)

**Exit criteria**: Full ecosystem with all platforms shipping, community contributions flowing.

---

## 11. Risk Register

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|-----------|--------|------------|
| 1 | **hledger parser scope creep** — edge cases consume months of development | High | High | Strict subset with documented boundaries. Fail loudly on unsupported features. Desktop hybrid path via hledger binary as escape hatch. |
| 2 | **WASM bundle too large** — slow web app initial load | Medium | High | Feature flags, lazy loading, 2 MB hard budget enforced in CI. Benchmark early (Phase 0). |
| 3 | **Sync conflicts frustrate users** — manual conflict resolution feels broken | Medium | High | Minimize conflict surface (append-mostly accounting data). Clear conflict UI with full context. Post-merge validation catches invariant violations. |
| 4 | **UniFFI async immaturity** — Swift integration rough edges | Medium | Medium | Swift wrapper layer abstracts UniFFI quirks. Monitor UniFFI roadmap. Fallback: C FFI if UniFFI is insufficient. |
| 5 | **Market data provider instability** — Yahoo Finance TOS changes, API breaks | High | Medium | Pluggable provider architecture. Multiple providers. User-supplied API keys for premium sources. Clear TOS documentation. |
| 6 | **Argon2id too slow on mobile** — vault unlock takes > 5s on older iPhones | Medium | Medium | Benchmark on target devices in Phase 0. Adjust mobile params. Consider progressive unlock (show cached data while deriving key). |
| 7 | **SQLite WASM performance** — sql.js overhead in browser | Low | Medium | Benchmark in Phase 5. Consider lighter alternatives (absurd-sql, wa-sqlite). Core queries are simple (no complex joins). |
| 8 | **iCloud Drive sync unreliability** — data loss from file coordination bugs | Medium | High | iCloud Drive is backup-only transport (single file), NOT event sync transport. API-based sync (Dropbox, WebDAV, server) for events. |
| 9 | **Single maintainer bottleneck** — project stalls | High | High | Good documentation, contribution guides, DCO. Phase 1 usable standalone. Community engagement early. |
| 10 | **Cryptographic implementation bugs** — security vulnerability | Low | Critical | Use audited RustCrypto crates only. Property-based tests. External security review before 1.0. No custom crypto primitives. |

---

## 12. Anti-Requirements

This project is explicitly **NOT**:

| Anti-Requirement | Rationale |
|-----------------|-----------|
| ❌ Bank API integration (Plaid/Yodlee) | Manual import is core. Automated import is a future community plugin, not a dependency. |
| ❌ Tax filing software | Tracks tax lots and generates reports. Does NOT file taxes or generate tax forms (8949, Schedule D). |
| ❌ Brokerage / trading platform | No order execution, no brokerage account linking. |
| ❌ Multi-user collaboration in MVP | Single-user vaults first. Shared vaults are a future feature (Phase 6+). |
| ❌ Crypto wallet | Tracks holdings. Does NOT hold private keys or execute blockchain transactions. |
| ❌ Mobile-first | CLI is MVP. Mobile follows in Phase 4. |
| ❌ SaaS product | Local-first, self-hosted. No hosted service in initial roadmap. |
| ❌ Jurisdiction-specific tax engine | Tax lot tracking is generic. Wash-sale rules, country-specific reporting are plugins, not core. |
| ❌ Drop-in hledger replacement | Compatible subset with import/export. Not feature-parity with hledger. |
| ❌ AI-powered categorization | Rule-based auto-categorization first. ML is a future enhancement, not a dependency. |

---

## 13. Open Questions (Requiring Prototyping)

| # | Question | How to Resolve | Phase |
|---|----------|----------------|-------|
| 1 | Argon2id parameters for mobile — what's the right memory/iteration tradeoff on iPhone 12 and above? | Benchmark on physical devices. Target < 2s unlock time. | Phase 0 |
| 2 | WASM bundle size — can ldgr-core `core` feature compile to < 2 MB compressed? | Build WASM target, measure with wasm-opt + gzip. Identify largest contributors. | Phase 0 |
| 3 | hledger parser — how many real-world journals use only the supported subset? | Collect sample journals from hledger community. Run `ldgr validate` against them. | Phase 1 |
| 4 | Sync conflict rate — how often do real users create conflicts? | Instrument sync in Phase 4. Collect anonymized conflict statistics. | Phase 4 |
| 5 | UniFFI overhead — is the FFI boundary a bottleneck for scrolling lists of transactions? | Benchmark UniFFI call overhead. Consider batching (fetch 50 transactions per call, not 1). | Phase 4 |
| 6 | sql.js memory usage — can a 10-year vault (100K+ transactions) fit in browser memory? | Generate synthetic data, load in sql.js, measure memory. | Phase 5 |
| 7 | Blob store API reliability — do Dropbox/WebDAV APIs provide reliable atomic read/write semantics for sync? | Build sync transport prototype, test with concurrent writes from 2 devices. | Phase 4 |

---

## Appendix: Reference Projects

| Project | What to Learn |
|---------|---------------|
| [pildora](https://github.com/kafkade/pildora) | Key hierarchy, vault encryption, UniFFI/WASM patterns, monorepo structure |
| [hledger](https://hledger.org/) | Transaction model, report types, query language, journal syntax |
| [ticker](https://github.com/achannarasappa/ticker) | Pluggable data sources (Yahoo, CoinGecko, CoinCap), bubbletea TUI |
| [tickrs](https://github.com/tarkah/tickrs) | ratatui TUI, Yahoo Finance API, candlestick/kagi charts |
| [Monarch Money](https://monarchmoney.com/) | Dashboard UX, net worth tracking, budget interface |
| [Standard Notes](https://standardnotes.com/) | Zero-knowledge sync, key management UX, multi-platform |
| [Bitwarden](https://bitwarden.com/) | Open-source ZK, self-hosted server, mobile + web + CLI |
| [beancount](https://beancount.github.io/) | Alternative plain-text accounting, plugin architecture, Fava web UI |
