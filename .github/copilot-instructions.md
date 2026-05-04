# Copilot Instructions for ldgr

## Project Overview

ldgr is a zero-knowledge, local-first personal finance application. It combines hledger-compatible double-entry bookkeeping with AES-256-GCM envelope encryption — the server (and any sync transport) never sees plaintext financial data.

**Stack**: Rust core library + CLI (clap/ratatui) + iOS/iPadOS (SwiftUI via UniFFI) + Web (Next.js + WASM via wasm-bindgen) + optional sync server (Axum, AGPL-3.0).

## Architecture

### Monorepo Layout

- `crates/ldgr-core/` — Shared Rust library: crypto, accounting engine, storage, sync, import/export, market data, loans, budget, goals. **No I/O, no networking** — pure computation only.
- `crates/ldgr-cli/` — CLI binary (clap commands + ratatui TUI). Only place in Rust that does HTTP (via reqwest).
- `crates/ldgr-server/` — Sync server (Axum). AGPL-3.0 licensed (has its own LICENSE file). Encrypted blob store — never decrypts user data.
- `bindings/swift/` — UniFFI-generated Swift bindings + idiomatic async/await wrapper layer.
- `apps/ios/` — SwiftUI app consuming the Swift bindings.
- `apps/web/` — Next.js app consuming ldgr-core via WASM.

### Key Design Constraints

1. **ldgr-core must have zero I/O dependencies.** No `reqwest`, no file system access, no platform APIs. All I/O happens in platform-specific code. This keeps the core testable (pure functions) and compilable to WASM.

2. **Market data flow**: Platform code (URLSession/fetch/reqwest) fetches HTTP → passes raw bytes to Rust core → core parses, validates, stores. Never do HTTP in ldgr-core.

3. **Encryption is always client-side.** The server and sync transports only see encrypted blobs. If you're adding a feature that touches data at rest or in transit, it must go through the vault encryption layer.

4. **Transactions are atomic.** A transaction with all its postings is a single unit for encryption, sync, and conflict resolution. Never create events for individual postings — always the full transaction.

5. **SQLite is the canonical store.** The vault is internally a SQLite database with versioned rows (soft deletes, version column). Sync uses a thin event outbox layer on top, NOT full event sourcing.

### Key Hierarchy (Crypto)

```
Password → Argon2id → MK → HKDF → MEK → wraps VK → wraps per-item IKs
```

- All key types implement `Zeroize`/`ZeroizeOnDrop`
- `Debug` impls must redact secret values
- Domain separation via AAD tags: `"ldgr-vault-wrap-v1"`, `"ldgr-item-wrap-v1"`, `"ldgr-recovery-wrap-v1"`
- Size-bucket padding before encryption (512B / 2KB / 8KB / 32KB)

### hledger Compatibility

ldgr supports a **strict subset** of hledger journal syntax (documented in `docs/journal-subset.md`). Unsupported features (includes, automated transactions, periodic transactions, lot notation) must produce a **clear error with line number** on import — never silently skip.

Export to hledger format is **one-way** (not bidirectional sync). The vault is the source of truth; hledger journals are an interchange format.

### Sync & Conflicts

- Sync events are **transaction-atomic** (full entity state per event, no partial updates)
- Events are batch-encrypted (not individually) for performance
- Conflicts on the same entity across devices require **user review** — no automatic last-write-wins for accounting data
- Post-merge validation must check double-entry invariants

## Conventions

### Decimal Arithmetic

All monetary amounts use `rust_decimal::Decimal`, stored as TEXT in SQLite. Never use floating-point for financial calculations.

### Error Handling

- Crypto failures must never expose key material in error messages
- Import errors must include file/line context
- Use `thiserror` for library errors, `anyhow` only in CLI/binary crates

### WASM Bundle Budget

The `core` WASM feature must stay under **2 MB compressed**. Feature flags in Cargo.toml control what's included:
- `core` = crypto + accounting + storage (always loaded)
- `sync`, `import-export`, `market`, `loans`, `budget`, `goals` = lazy-loaded

### UniFFI / Swift

UniFFI generates callback-based APIs. The `bindings/swift/Sources/LdgrSwift/` wrapper layer converts these to idiomatic Swift async/await using `withCheckedThrowingContinuation`.

### Licensing

Everything is Apache-2.0 except `crates/ldgr-server/` which is AGPL-3.0. Don't move server code into ldgr-core or vice versa without considering license implications.

## Build & Test

```sh
# Build all crates
cargo build

# Run all tests
cargo test

# Run a single test
cargo test -p ldgr-core test_name

# Run tests for a specific module
cargo test -p ldgr-core crypto::

# Clippy lints
cargo clippy --workspace --all-targets

# Format check
cargo fmt --check

# Build WASM (requires wasm-pack)
wasm-pack build crates/ldgr-core --target web --features core

# Check WASM bundle size
wasm-pack build crates/ldgr-core --target web --features core --release
gzip -c crates/ldgr-core/pkg/ldgr_core_bg.wasm | wc -c
```

## ADRs

Architecture Decision Records live in `docs/adr/`. Read them before making changes to:
- Source of truth model (ADR-001)
- Parser scope / hledger compatibility (ADR-002)
- Sync and conflict resolution (ADR-003)
- Data model (ADR-004)
- Platform boundaries — what goes in Rust vs Swift vs TypeScript (ADR-005)
- Licensing (ADR-006)

## Git Policy

**Never execute Git commands that modify history or submit code.** This includes `git commit`, `git push`, `git rebase`, `git merge`, `git reset`, `git cherry-pick`, `git revert`, and `git tag`. Read-only commands like `git status`, `git diff`, `git log`, and `git branch` are fine. The maintainer must always review and commit changes themselves.

## CI / Infrastructure Dependency

**Branch protection for this repo is managed via Terraform in `kafkade/github-infra` (`repo_ldgr.tf`).** The `required_status_checks` list must match the job names in `.github/workflows/ci.yml`. If you rename, add, or remove CI jobs that are used as merge gates, the corresponding IaC config must be updated or PRs will be permanently blocked. Always flag this when proposing workflow changes.

## PR Title Format

Use conventional commits: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`. For multi-component changes, include the primary component: `feat(crypto): add vault key wrapping`.

## Reference Documents

The full product roadmap is in `docs/roadmap.md`. The architecture document with all decisions, data model, and platform designs is in `ldgr-architecture.md`.
