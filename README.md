# ldgr

**Zero-knowledge bookkeeping.**

An open-source, privacy-first personal finance system built on plain-text accounting principles. ldgr combines the rigor of double-entry bookkeeping (hledger-compatible) with AES-256-GCM envelope encryption — no server, sync transport, or third party ever sees your plaintext financial data.

## Features

- **Zero-knowledge encryption** — client-side AES-256-GCM with Argon2id key derivation. The vault key hierarchy ensures data is encrypted before it leaves your device.
- **Double-entry bookkeeping** — proper accounting with hierarchical accounts, balance assertions, and multi-currency support.
- **Recovery key** — 24-word Crockford Base32 emergency key generated at vault creation. Lost password + lost recovery key = unrecoverable (by design).
- **Local-first** — SQLite-backed storage with versioned rows. Works fully offline. Sync is optional.
- **Self-hosted sync server** — optional Axum-based relay server with SRP-6a zero-knowledge authentication. The server stores only encrypted blobs — it never sees plaintext financial data. Docker image included.
- **Multi-platform** — Rust core library with CLI (clap + ratatui), iOS/iPadOS (SwiftUI via UniFFI), and web (Next.js + WASM) frontends.
- **hledger-compatible** — import from and export to hledger journal format. Use `hledger` for reporting if you prefer.
- **Investment tracking** — value holdings at market prices for net worth calculations. Market data from Yahoo Finance, CoinGecko (crypto), and ECB (forex) — all free, no API keys required.

> **Note**: ldgr is a **net worth tracker**, not a trading platform. Market data
> is used to value your investment holdings as part of the overall financial
> picture. For investment decisions, use specialized tools (your brokerage
> platform, Bloomberg, etc.).

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

# Add accounts and transactions
ldgr accounts add Assets:Checking:Chase
ldgr accounts add Expenses:Food
ldgr add --date 2024-01-15 --description "Groceries" \
  --posting "Expenses:Food  42.50 USD" \
  --posting "Assets:Checking:Chase  -42.50 USD"

# View reports
ldgr balance
ldgr register
ldgr incomestatement
ldgr balancesheet

# Import from bank exports
ldgr import statement.csv --profile chase
ldgr import statement.ofx

# Export for hledger
ldgr export --format hledger | hledger balance

# Lock when done
ldgr lock
```

## Commands

| Command | Description |
| --- | --- |
| `ldgr init` | Create a new encrypted vault |
| `ldgr unlock` | Unlock vault with master password |
| `ldgr lock` | Lock vault (clear session) |
| `ldgr status` | Show vault path, version, and lock state |
| `ldgr accounts` | List all accounts |
| `ldgr accounts add <name>` | Create account (auto-detects type from name) |
| `ldgr accounts rename <old> <new>` | Rename an account |
| `ldgr add` | Add a transaction (interactive or with flags) |
| `ldgr delete <id>` | Soft-delete a transaction |
| `ldgr balance [query]` | Hierarchical account balances |
| `ldgr register [query]` | Chronological register with running balance |
| `ldgr incomestatement [query]` | Income statement (Revenue - Expenses) |
| `ldgr balancesheet [query]` | Balance sheet (Assets - Liabilities = Equity) |
| `ldgr import <file>` | Import CSV or OFX/QFX bank exports |
| `ldgr export --format <fmt>` | Export to hledger, CSV, or JSON |
| `ldgr validate <file>` | Check journal importability |
| `ldgr reconcile <account>` | Interactive reconciliation |
| `ldgr rules` | Manage import auto-categorization rules |

## Self-Hosted Sync Server

The optional sync server (`crates/ldgr-server/`) is an encrypted blob relay — it
stores and serves encrypted blobs but never decrypts them.

```sh
# Run with Docker
docker build -t ldgr-server -f crates/ldgr-server/Dockerfile .
docker run -p 8080:8080 -v ldgr-data:/data ldgr-server

# Or run directly
cargo run -p ldgr-server
```

**Configuration** (environment variables):

| Variable | Default | Description |
| --- | --- | --- |
| `LDGR_BIND_ADDR` | `127.0.0.1:8080` | Listen address |
| `LDGR_DB_PATH` | `ldgr-server.db` | SQLite database path |
| `LDGR_SESSION_TTL_HOURS` | `720` | Session lifetime (30 days) |
| `LDGR_MAX_BLOB_BYTES` | `52428800` | Max blob size (50 MB) |
| `LDGR_RELAY_TTL_MINUTES` | `10` | Key exchange relay offer TTL |

**API endpoints**: Register, login (SRP-6a), vault management, encrypted batch
and snapshot CRUD, device registration, and key exchange relay. See the
[architecture doc](docs/ldgr-architecture.md) for details.

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

- [How is my data protected?](docs/security/vault-overview.md) — plain-language overview of ldgr's encryption, no technical background needed
- [Architecture & Roadmap](docs/ldgr-architecture.md) — full system design, ADRs, data model
- [Roadmap](docs/roadmap.md) — phased development plan
- [ADRs](docs/adr/) — architecture decision records, including [ADR-008: Self-Hosting + Two-Secret Account Auth](docs/adr/008-self-hosting-and-account-auth.md)

## License

All components are licensed under [Apache-2.0](LICENSE), except the sync server (`crates/ldgr-server/`) which is licensed under [AGPL-3.0](crates/ldgr-server/LICENSE).

See [ADR-006](docs/ldgr-architecture.md#adr-006-licensing--apache-20-with-agpl-server--dco) for the licensing rationale.

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting a pull request. All contributions require a DCO (Developer Certificate of Origin) sign-off.
