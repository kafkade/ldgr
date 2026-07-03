# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cloudflare Worker market data proxy (`infra/market-proxy`) at `api.ldgr.dev/market/`: a shared caching proxy in front of Yahoo Finance, CoinGecko, and the ECB that serves many users the same symbols from one cached upstream response (ADR-007). Routes for quotes (`/quote`), crypto (`/crypto`), forex (`/forex`), historical OHLCV (`/historical`), and a `/health` check; symbol lists are normalized and sorted so `AAPL,MSFT` and `MSFT,AAPL` share a cache key; responses are cached in Cloudflare KV with per-type TTLs (quotes/crypto 15 min, historical/forex 24 hr); concurrent misses are de-duplicated to a single upstream fetch and upstream requests are throttled to ~1/sec per provider; CORS headers and an `X-Cache: HIT|MISS` header are returned for the web app. The proxy only ever handles public market data and clients fall back to direct provider requests when it is unavailable
- Native macOS window and sidebar interface: on the Mac the app now presents a `NavigationSplitView` sidebar (Dashboard, Transactions, Accounts, Investments, Budget) with a resizable list/detail layout instead of the iPhone-style tab bar, plus a macOS menu bar with commands and keyboard shortcuts — New Transaction (⌘N), Import… (⌘I), New Window (⇧⌘N), Sync Now (⌘R), and Lock Vault (⌘L) — and multi-window support. The navigation shell is Mac-specific while the underlying views, view models, and encrypted vault are shared with iOS. (Import currently opens a file picker but journal import is not yet wired end-to-end.)
- macOS unlock with Touch ID, a paired Apple Watch, or password fallback: the native macOS app stores the vault session key in the macOS Keychain (data-protection keychain, device-only) gated by user presence, unlocks biometrically on Touch ID Macs, and falls back to the account password on non-biometric Macs. Session material is cleared on lock and the master password is never persisted, matching the iOS biometric unlock experience
- macOS and watchOS slices in the Swift binding XCFramework (`ldgr_ffiFFI.xcframework`): the UniFFI framework now vends iOS (device), iOS simulator (arm64 + x86_64), macOS (arm64 + x86_64), watchOS (device), and watchOS simulator (arm64) slices, and the `LdgrSwift` package declares macOS and watchOS as supported platforms — so the native macOS app and the watchOS target can link the shared Rust core
- Native macOS app: a `ldgr-macos` application target in the Xcode project that runs the same SwiftUI interface as the iPhone and iPad app on the Mac, reusing the shared views and the encrypted local-first vault. Platform-specific behavior (clipboard, toolbars, backgrounds, Apple Watch connectivity) is adapted per platform so the Mac build stays feature-consistent with iOS while dropping integrations that don't apply on macOS
- Two-secret (2SKD) sign-up / sign-in with an **Emergency Kit** across every client — CLI, web, and iOS/iPadOS (ADR-008). When a server advertises two-secret auth, creating an account generates an **Account Secret Key** and shows a save-once Emergency Kit (boxed text + scannable QR; terminal QR and optional `0600` file export on the CLI, share/print in the apps); adding a **new device** then requires that Secret Key (typed or scanned) alongside the password, while existing devices sign in with the password alone. The vault still opens offline with the master password only — the Secret Key is auth/sync-only. Clients discover the mode via `GET /server/info` and fall back to single-secret (password-only) SRP-6a automatically. The account id needed for key derivation is generated client-side at sign-up (UUIDv7), persisted by the server, and returned at `login/init`, so the server database still can't be brute-forced offline. The Secret Key is stored per-platform in secure storage (CLI `sync-credentials.json` `0600`, iOS Keychain, WASM-guarded/vault-encrypted storage) and the master password is never persisted
- Web admin panel (`apps/web` `/admin`) for user and server management against a self-hosted `ldgr-server`: SRP sign-in (reusing the WASM sync client, so the password never leaves the browser and non-admins are rejected), user management (enable/disable, role, per-user quota, delete), invite issuance/revocation with one-time token display, server settings (registration policy, default quota, max blob size), a per-user storage/usage dashboard, and a server info/version view — kept in the Apache-2.0 web app talking to the headless AGPL server over JSON (ADR-008 §7)
- Immich-style docker-compose self-hosting bundle at the repo root (`docker-compose.yml`, `.env.example`, `Caddyfile`): `cp .env.example .env && docker compose up -d` yields a running server with a persistent named volume and a passing `/health` healthcheck; pulls the published GHCR image by default with a commented local-build override, and an optional Caddy `tls` profile for automatic HTTPS
- Multi-arch (amd64/arm64) `ldgr-server` container images published to GHCR (`ghcr.io/kafkade/ldgr-server`) on release tags via a dedicated CI workflow, tagged with the version, `major.minor`, `major`, and `latest`
- Self-hosting guide (`docs/self-hosting.md`) covering one-command deploy, full env-var reference, TLS with Caddy (plus nginx/Traefik notes), upgrades, volume backup/restore, first-run admin onboarding and the web admin panel, registration policy, adding/managing users, and the 1Password-style two-secret account model (master password, vault recovery key, and Account Secret Key + Emergency Kit) with a threat-model recap
- Public, unauthenticated server-discovery endpoints (`GET /api/v1/server/info` and `GET /api/v1/server/ping`) so clients can validate a server URL, read the sync/auth protocol version, the effective registration policy, and capability flags (e.g. two-secret auth) before sign-in; instance label configurable via `LDGR_SERVER_NAME`
- SRP-6a client primitives and transport-agnostic server sync protocol types in `ldgr-core` (registration verifier generation, login proof computation, session key derivation, and serde request/response types for every server endpoint), reused by all platform clients without performing any I/O
- `ServerSyncClient` that orchestrates the auth handshake and encrypted batch/snapshot/device/relay sync over an injected platform HTTP callback
- Self-hosted `ldgr-server` sync transport in the CLI: `ldgr sync setup` now offers the server as a provider (SRP-6a sign-in/registration, vault creation, and device registration), and `ldgr sync push` / `ldgr sync pull` move encrypted batches and snapshots to and from your own server over HTTPS
- Two-secret (2SKD) sign-in support in the sync client (`ServerSyncClient::register_2skd` / `login_2skd`): registering and logging in with both the password-derived auth key and the account Secret Key, advertising the `srp-2skd-v1` scheme while keeping single-secret registration wire-compatible
- Persistent client-side market price cache (SQLite) with per-type TTLs (quotes 15 min, historical 24 hr) so cached prices survive CLI restarts and avoid repeat network requests within TTL
- `ldgr cache status` and `ldgr cache clear` commands to inspect hit rate / entry count and flush cached prices
- Apple Watch companion app: read-only glances for net worth, portfolio, and monthly spending
- Watch complications (WidgetKit): net worth, daily spend, and portfolio widgets for watch faces
- WatchConnectivity sync: iPhone sends pre-computed financial summaries to Watch without exposing vault data
- iOS home screen widgets: net worth (small/medium), monthly spending (medium), and portfolio (medium)
- Siri Shortcuts: query net worth, check monthly spending, and add expense via App Intents
- Widget data cache cleared on vault lock to prevent financial data exposure on the lock screen
- Market data provider registry with metadata, discovery, and community provider support
- Provider development guide with step-by-step implementation walkthrough and TOS guidance
- Example provider crate (`examples/ldgr-provider-example/`) as a template for community providers
- Loan tracking module: fixed and variable rate amortization schedules with month-by-month breakdown
- Payoff projections with extra payments and biweekly payment scenarios
- Refinance comparison with break-even analysis and month-by-month cost simulation
- Payment auto-split into principal and interest portions for ledger posting
- Self-hosted sync server with SRP-6a zero-knowledge authentication and encrypted blob storage
- Server API for vault management, encrypted batch and snapshot sync, device registration, and key exchange relay
- Docker image for self-hosted deployment with configurable bind address, session TTL, and blob size limits
- WASM build pipeline with wasm-bindgen API for vault crypto, journal parsing, and balance/register reports in the browser
- sql.js integration for client-side SQLite storage in the web app
- CI bundle size enforcement: WASM core module must stay under 2 MB compressed
- Next.js web app with vault creation, password unlock, and encrypted local storage via IndexedDB
- Web dashboard with net worth, account stats, recent transactions, and expense breakdown
- Web transaction management: list with search, month grouping, add/delete transactions
- Web account management: grouped by type with balances and add form
- Web investments view: portfolio holdings by commodity with allocation chart
- Web budget view: expense category breakdown with progress bars
- Dark/light theme with system preference detection and manual toggle
- Offline support via service worker caching of WASM and static assets
- Responsive layout: sidebar navigation on desktop, tab bar on mobile
- Vault item replacement and clearing in core crypto API for efficient web persistence
- CLI theming system with five built-in themes (default, light, solarized, nord, dracula) and custom theme support via config
- `ldgr config` subcommand: `set`, `get`, and `list-themes` for managing CLI settings
- Live theme reload in TUI views (watchlist, portfolio, chart) without restarting the application
- Web theme preference with system/light/dark options and live system preference tracking
- Vault format expert specification (`docs/security/vault-format-spec.md`): byte-precise binary format definition for independent re-implementation and security audit
- Published vault format test vectors (`docs/security/test-vectors.md`) with binary fixtures and a CI conformance test, so third-party implementations can verify byte-for-byte compatibility with the v1 vault format
- Account Secret Key (`A1-…` format): a high-entropy key combined with your password during server sign-in, so a stolen password alone cannot authenticate to the sync server
- Account Emergency Kit: a printable/QR-ready artifact bundling your sign-in address, account email, and account Secret Key for fast sign-in on a new device (with optional inclusion of the vault recovery key); core can generate the kit and parse a scanned/typed kit back to onboarding values
- Two-secret key derivation for the sync server's SRP-6a verifier — server authentication now requires both the password and the account Secret Key, while the local vault still opens with the password (or vault recovery key) alone
- Multi-user accounts for the self-hosted sync server: email sign-in identity, admin/user roles, and active/disabled account status
- Server registration policy (`LDGR_REGISTRATION`): `open`, `invite-only` (default), or `admin-only`, with invite-token redemption for invite-only instances
- First-run admin bootstrap for the sync server: seed an admin from `LDGR_ADMIN_EMAIL` (recommended for docker-compose), or the first account to register becomes the admin
- Per-user storage quotas on the sync server, with a configurable server default (`LDGR_DEFAULT_QUOTA_BYTES`, default 1 GiB) and optional per-account override; uploads exceeding the quota are rejected
- Admin API for the sync server (`/api/v1/admin`, admin-only, JSON): list/create/disable/enable/delete users, change roles and storage quotas, and view per-user usage
- Admin invite management: issue, list, and revoke invite tokens for invite-only instances (issued tokens redeem through the normal registration flow)
- Runtime-updatable server settings via the admin API: registration policy, default storage quota, and max blob size are now persisted and editable without a restart, with environment variables providing the initial defaults
- Server stats endpoint for admins reporting account count and per-user / total storage usage
- Last-admin protection: the final active administrator cannot be disabled, demoted, or deleted
- Swift/UniFFI server-sync bindings: an `LdgrSyncClient` that drives SRP-6a sign-in/registration (single-secret and two-secret), vault creation, and encrypted batch/snapshot/device sync from Swift, with all key derivation kept in Rust so no key material crosses the binding boundary
- Platform-provided async HTTP transport seam (`FfiHttpSender`) with a ready-to-use `URLSessionHTTPSender` for iOS, letting the app supply networking while encrypted blobs move opaquely through the bindings
- Multi-device sync batch-blob pipeline in `ldgr-core`: compose pending changes into a single encrypted blob (`export_pending_batch`) and apply a downloaded blob back into the local vault (`ingest_batch`), with full-state transaction-atomic events, version-gated upsert-by-id, soft-delete propagation, idempotent re-ingest, and concurrent edits to the same entity surfaced as conflicts for review rather than silently overwritten
- iOS app now syncs with a self-hosted `ldgr-server`: sign in or create an account from Sync settings, then push and pull encrypted change batches over HTTPS with a persisted incremental cursor, and review concurrent-edit conflicts — replacing the previous in-app placeholder transport
- Swift/UniFFI vault bindings expose the batch-blob pipeline (`exportPendingBatch` / `ingestBatch`) so platform apps can compose and apply encrypted sync batches directly from an unlocked vault
- Web app now syncs with a self-hosted `ldgr-server`: register or sign in via SRP-6a from vault settings, create or select a remote vault, and push/pull encrypted change batches over HTTPS — the server only ever sees encrypted blobs, never plaintext or key material (key derivation and batch sealing stay in WASM)
- Web sync conflict review: concurrent edits to the same account or transaction are surfaced for review (keep local or keep remote) instead of being silently overwritten
- CLI now syncs end-to-end with a self-hosted `ldgr-server`: `ldgr sync push` exports pending changes through the encrypted batch-blob pipeline and `ldgr sync pull` applies downloaded batches into the local vault, materializing accounts and transactions and surfacing concurrent-edit conflicts for review — replacing the previous file-staging placeholder that uploaded nothing and never applied pulled changes
- `ldgr sync resolve` to review and resolve pending sync conflicts (keep local), and `ldgr sync status` now reports the pending-push event count and unresolved conflict count
- `ldgr sync resolve` can now keep the *remote* version of a conflict, not just the local one: the remote change is re-applied to your vault and re-broadcast so every device converges on it (previously choosing "remote" was unsupported)
- Financial goals are now persisted in the vault as a versioned entity (soft-delete + optimistic-concurrency `version`), so goals survive restarts and are ready to sync — previously goals existed only in memory
- Observed price points are now persisted in the vault as a versioned entity (soft-delete + optimistic-concurrency `version`), giving prices a canonical store distinct from the transient market-data HTTP cache so they survive restarts and are ready to sync
- Budget definitions are now persisted in the vault as a versioned entity (soft-delete + optimistic-concurrency `version`), with each budget's category allocations stored in deterministic order, so budgets survive restarts and are ready to sync — previously budgets existed only in memory
- Financial goals now sync across your devices: creating, editing, and deleting a goal propagates through the encrypted batch-blob pipeline like accounts and transactions, with full-state events, version-gated upsert-by-id, soft-delete propagation, and concurrent-edit conflicts surfaced for review instead of silently overwritten
- Budgets now sync across devices through the encrypted batch-blob pipeline: a budget and its ordered category allocations move as a single transaction-atomic event, are reproduced field-for-field (including allocation order) on other devices, and concurrent edits to the same budget surface as conflicts for review instead of being silently overwritten
- Observed price points now sync across your devices through the encrypted batch-blob pipeline (full-state, transaction-atomic events with version-gated upsert-by-id and soft-delete propagation), and concurrent edits to the same price are surfaced as conflicts for review rather than silently overwritten (ADR-003)

### Fixed

- `ldgr-server` router now uses axum 0.8 `{param}` path syntax; the previous `:param` syntax panicked at router construction under axum 0.8
- CLI vault writes (add account, rename account, add/import/delete transaction) now record sync-outbox events, so locally created changes are actually included in the next push instead of being silently skipped
- iOS vault writes (add account, add/delete transaction) now record sync-outbox events, so locally created changes are actually included in the next push instead of being silently skipped
- Web vault writes (add account, add/delete transaction) now record sync-outbox events, so locally created changes are actually included in the next push instead of being silently skipped (the `sync_events`/`sync_state` tables were previously dead scaffolding)
- Resolving a sync conflict as "keep remote" now actually re-applies the remote change (via the Swift/iOS bindings it previously only marked the conflict resolved without materializing the remote version)
- Sync against a self-hosted `ldgr-server` now lists and pulls every encrypted batch and snapshot in large vaults: blob listings were previously capped at a single server page (~1000 entries), so a vault with more than one page of batches or snapshots would silently drop — and never pull — the overflow. The client now follows the server's continuation cursor across all pages

## [1.2.0] - 2026-05-30

### Added

- Background sync infrastructure: event outbox, conflict storage, Lamport clock, and device identity
- Sync-aware account and transaction mutations that record outbox events atomically
- Sync status dashboard in iOS app showing pending changes, conflicts, and last sync time
- Conflict resolution UI with side-by-side local vs remote comparison and Keep Local / Keep Remote actions
- Sync status indicator in toolbar (green checkmark, blue pending, orange conflict warning)

## [1.1.0] - 2026-05-30

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
