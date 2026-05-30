# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- iOS app: tab bar (iPhone) and sidebar (iPad) navigation with Dashboard, Transactions, Accounts, Investments, and Budget tabs
- Dashboard tab with net worth by commodity, recent transactions, and expense breakdown
- Transaction list with search, status filter, swipe-to-delete, and add/correct transaction form
- Accounts tab grouped by type with balances and per-account transaction register
- Investments tab with portfolio holdings and allocation chart
- Budget tab with current-month expense category progress bars
- Pull-to-refresh on all data views to reload from encrypted vault
- Shared vault data store so mutations in one tab update all tabs instantly
- Real-time watchlist TUI: `ldgr watch [symbols...]` with auto-refresh, sparklines, sort, and search
- Interactive charts: line and candlestick views with timeframe selection, zoom, volume bars, and moving averages
- Portfolio view: `ldgr portfolio` showing holdings, market values, gain/loss, and allocation percentages
- Blob store sync transports for Dropbox (OAuth2 PKCE) and WebDAV with retry middleware
- CLI sync commands: `ldgr sync setup`, `ldgr sync push`, `ldgr sync pull`, `ldgr sync status`
- UniFFI bindings with Swift async wrapper for iOS/macOS integration
- XCFramework build pipeline for cross-compiling to iOS and simulator targets
- iOS app with vault creation, password unlock, and account/balance dashboard
- Face ID and Touch ID biometric unlock via Keychain-stored session key
- Auto-lock on app background with configurable timeout (immediate / 1 min / 5 min / 15 min)
- Privacy overlay hides vault content in the app switcher
- Recovery key display with copy and share during vault creation
- Session key export/import in FFI for biometric unlock without password re-derivation

## [1.0.0] - 2026-05-06

### Added

- Budgeting module with envelope (rollover) and zero-based methods
- Budget vs actual comparison with over-budget detection and percentage tracking
- Recurring transaction detection: subscriptions, variable recurring, and income patterns
- Missing recurring transaction alerts
- Financial goals tracking with savings, debt payoff, investment, and emergency fund types
- Goal projections: linear timeline, what-if scenarios, required monthly contribution
- Sync event generation with Lamport clocks, vector clocks, and batch serialization
- Conflict detection for concurrent entity modifications across devices with user-review resolution
- Snapshot compaction for efficient new-device onboarding with configurable retention policy
- Device onboarding via X25519 key exchange with QR payload and MITM-prevention verification code
- CoinGecko market data provider for cryptocurrency prices (no API key required)
- ECB exchange rate provider for EUR-based forex rates (no API key, official government data)
- Client-side market data cache with configurable TTL (15 min quotes, 24 hr historical)
- Provider chain with automatic routing by asset class and fallback on failure

## [0.1.0] — 2026-05-04

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
