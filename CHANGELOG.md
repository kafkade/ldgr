# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cargo workspace with three crates: `ldgr-core` (library), `ldgr-cli` (binary), `ldgr-server` (sync server)
- CLI skeleton with `init`, `unlock`, `lock`, and `status` subcommands (stubs)
- Core library with `crypto` and `storage` module placeholders
- Feature flags for WASM bundle control (`core`, `sync`, `import-export`, `market`, `loans`, `budget`, `goals`)
- Key hierarchy: Argon2id password derivation, HKDF domain-separated key derivation, and AES-256-GCM key wrapping with recovery key support
- Per-item envelope encryption with size-bucket padding (512 B / 2 KB / 8 KB / 32 KB)
- CI pipeline: build, test, clippy, formatting, and WASM smoke test on every PR
- Release pipeline: multi-platform binary builds and GitHub Releases on tag push
- Vault binary file format with `LDGR` magic bytes, versioned header, and encrypted metadata
- Vault operations: create, open (unlock with password), serialize (save), and validate
- Recovery key generation at vault creation with Crockford Base32 human-readable display
- Vault recovery flow: unlock with recovery key and set a new password
- Password change for unlocked vaults (re-wraps vault key, items untouched)
- SQLite storage layer with schema migration mechanism (`sqlite` feature flag)
- Account CRUD with hierarchical names, optimistic concurrency, and soft delete
- Transaction and posting CRUD with atomic writes and version tracking
- CLI commands: `ldgr init`, `ldgr unlock`, `ldgr lock`, `ldgr status`
- `--vault <PATH>` flag to specify a custom vault file path
- Session-based vault unlock with configurable timeout (`--timeout`, default 15 min)
- Recovery key displayed in a bordered box during vault creation
- SQLite database auto-initialized alongside the vault on `ldgr init`
- WASM-optimized Argon2id parameter preset (64 MB, 3 iterations, single-threaded)
- hledger journal parser supporting transactions, postings, amounts, balance assertions, tags, comments, and directives
- Documented hledger syntax subset specification (`docs/journal-subset.md`)
- Account management: `ldgr accounts`, `ldgr accounts add`, `ldgr accounts rename`, `ldgr accounts delete`
- Auto-detect account type from name prefix (e.g., `Assets:*` → Asset) with auto-parent creation
- Transaction entry: `ldgr add` (interactive and non-interactive with `--date`, `--description`, `--posting`)
- Double-entry validation: postings must sum to zero, at most one auto-balance posting
- Transaction deletion: `ldgr delete <id>` with confirmation prompt (`--force` to skip)
- CSV import with configurable column mapping profiles (`ldgr import <file.csv> --profile <name>`)
- Auto-delimiter detection (comma, semicolon, tab) with quoted field and BOM support
- Import rules engine for auto-categorization (`ldgr rules add --pattern "WHOLE FOODS" --account "Expenses:Food"`)
- Rule matching: case-insensitive substring, exact, and starts-with modes with priority ordering
- Balance report: `ldgr balance` with hierarchical account tree, subtotals, and multi-commodity support
- Register report: `ldgr register` with chronological transaction list and running balance
- Query filters for reports: `--begin`, `--end`, and account name substring filtering
- Report output formats: `--output table|json|csv` for balance and register
- Income statement report: `ldgr incomestatement` (alias `is`) showing Revenue - Expenses = Net Income
- Balance sheet report: `ldgr balancesheet` (alias `bs`) showing Assets, Liabilities, and Equity
- Query language for filtering: `acct:`, `desc:`, `date:`, `amt:>`, `amt:<`, `tag:`, `not:` with AND composition
- Journal validation tool: `ldgr validate <file>` checks importability, reports errors with line numbers, shows statistics on success
- OFX/QFX import parser extracting date, amount, payee, memo, and FITID from bank exports
- Import deduplication with three match levels: exact (FITID), strong (date + amount + payee similarity), weak (nearby date + amount)
- Interactive account reconciliation: `ldgr reconcile <account>` with statement balance matching, running totals, and partial save/resume
- Investment lot tracking with buy/sell/partial disposal and unrealized gain/loss
- Cost basis methods: FIFO, LIFO, Average Cost, and Specific Identification for lot disposal
- Short-term vs long-term holding period classification (365-day threshold)
- Pluggable market data provider trait (I/O-free: builds URLs and parses responses)
- Yahoo Finance provider: current quotes and historical OHLCV for stocks, ETFs, crypto, forex
- Net worth tracking with breakdown by liquid assets, investments, and liabilities
- Cash flow report grouped by operating, investing, and financing activities
- Trial balance report with debit/credit totals and balance verification
- Export to hledger journal, CSV, and JSON: `ldgr export --format hledger|csv|json` with query filters
