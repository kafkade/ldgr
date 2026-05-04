# ldgr

**Zero-knowledge bookkeeping.**

An open-source, privacy-first personal finance system built on plain-text accounting principles. ldgr combines the rigor of double-entry bookkeeping (hledger-compatible) with AES-256-GCM envelope encryption — no server, sync transport, or third party ever sees your plaintext financial data.

## Features

- **Zero-knowledge encryption** — client-side AES-256-GCM with Argon2id key derivation. The vault key hierarchy ensures data is encrypted before it leaves your device.
- **Double-entry bookkeeping** — proper accounting with hierarchical accounts, balance assertions, and multi-currency support.
- **Recovery key** — 24-word Crockford Base32 emergency key generated at vault creation. Lost password + lost recovery key = unrecoverable (by design).
- **Local-first** — SQLite-backed storage with versioned rows. Works fully offline. Sync is optional.
- **Multi-platform** — Rust core library with CLI (clap + ratatui), iOS/iPadOS (SwiftUI via UniFFI), and web (Next.js + WASM) frontends.
- **hledger-compatible** — import from and export to hledger journal format. Use `hledger` for reporting if you prefer.

## Quick Start

```sh
# Install from source
cargo install --path crates/ldgr-cli

# Create a new vault
ldgr init
# → Prompts for master password
# → Displays recovery key (write it down!)
# → Creates vault at ~/.ldgr/vault.ldgr

# Unlock the vault
ldgr unlock

# Check vault status
ldgr status

# Lock when done
ldgr lock
```

## Architecture

ldgr is a monorepo with a shared Rust core and platform-specific frontends:

```text
crates/ldgr-core/    Shared Rust library (crypto, accounting, storage, sync)
crates/ldgr-cli/     CLI binary (clap + ratatui TUI)
crates/ldgr-server/  Sync server (Axum, AGPL-3.0 licensed)
bindings/swift/      UniFFI-generated Swift bindings
apps/ios/            SwiftUI app
apps/web/            Next.js + WASM app
```

**Key design constraint**: `ldgr-core` has **zero I/O dependencies**. No networking, no file system access, no platform APIs. All I/O happens in platform-specific code. This keeps the core testable, deterministic, and compilable to WASM.

### Key Hierarchy

```text
Password → Argon2id → Master Key → HKDF → Master Encryption Key → wraps Vault Key → wraps Item Keys
                                         → Recovery Key (alternate path to Vault Key)
```

All key types implement `Zeroize` and `ZeroizeOnDrop`. Debug implementations redact secret values.

### Vault Format

The vault uses a custom binary format (`LDGR` magic bytes) with:

- Argon2id KDF parameters in the header (upgradeable on password change)
- Vault key wrapped by both the password-derived MEK and the recovery key
- Per-item envelope encryption with size-bucket padding (512 B / 2 KB / 8 KB / 32 KB)
- Domain-separated AAD tags for each wrapping operation

## Building from Source

```sh
# Prerequisites: Rust 1.85+
cargo build --workspace

# Run tests
cargo test --workspace

# Run clippy
cargo clippy --workspace --all-targets

# Check formatting
cargo fmt --check
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for development workflow details.

## Documentation

- [Architecture & Roadmap](docs/ldgr-architecture.md) — full system design, ADRs, data model
- [Roadmap](docs/roadmap.md) — phased development plan
- [ADRs](docs/adr/) — architecture decision records

## License

All components are licensed under [Apache-2.0](LICENSE), except the sync server (`crates/ldgr-server/`) which is licensed under [AGPL-3.0](crates/ldgr-server/LICENSE).

See [ADR-006](docs/ldgr-architecture.md#adr-006-licensing--apache-20-with-agpl-server--dco) for the licensing rationale.

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting a pull request. All contributions require a DCO (Developer Certificate of Origin) sign-off.
