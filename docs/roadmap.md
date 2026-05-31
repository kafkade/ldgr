# ldgr Roadmap

> **Zero-knowledge bookkeeping.**

Each phase produces a usable, shippable product increment. Phases are sequential — each builds on the previous — but individual issues within a phase may be worked in parallel.

---

## Phase 0 — Foundation

**Goal**: Buildable monorepo with crypto module and basic vault operations.

### Deliverables

- Monorepo structure with Cargo workspace and CI/CD (GitHub Actions)
- `ldgr-core` crate: crypto module (key hierarchy, Argon2id, AES-256-GCM, vault encrypt/decrypt)
- Vault file format implementation (create, open, lock, unlock)
- Recovery key generation and emergency kit
- SQLite schema definition (core tables: accounts, transactions, postings, commodities, prices)
- CLI commands: `ldgr init`, `ldgr unlock`, `ldgr lock`, `ldgr status`
- Property-based tests for crypto module
- Documentation: README, CONTRIBUTING.md, DCO, LICENSE
- Benchmark: Argon2id parameter tuning across platforms

**Exit criteria**: Can create a vault, unlock it, lock it, and verify crypto round-trips correctly.

---

## Phase 1 — CLI Ledger (MVP)

**Goal**: Usable single-device ledger. A user can track their finances entirely via CLI.

### Deliverables

- hledger journal parser (strict subset v1.0, see ADR-002)
- Document supported hledger syntax spec
- Account management (create, list, rename)
- Transaction entry and editing (`ldgr add`, `ldgr edit`, `ldgr delete`)
- CSV import with configurable column mapping
- Import rules engine (payee → account auto-categorization)
- Reports: `balance`, `register`, `incomestatement`, `balancesheet`
- Multi-currency support with manual price entries
- Query language (account, description, date, amount, tag filters)
- Journal validation tool (`ldgr validate`)
- Output formatting: `--output table|json|csv`
- Conformance tests against hledger binary output

**Exit criteria**: A user can import their bank CSV, categorize transactions, and generate a balance report — all encrypted at rest.

---

## Phase 2 — Import, Investments & Net Worth

**Goal**: Full import pipeline and investment tracking.

### Deliverables

- OFX/QFX import support
- Import deduplication (fuzzy matching: date ±2 days, exact amount, payee similarity)
- Reconciliation workflow (`ldgr reconcile`)
- Investment lot tracking (create lots on buy, dispose on sell)
- Cost basis methods: FIFO, LIFO, Specific Identification, Average Cost
- Capital gains calculation (realized/unrealized, short-term/long-term)
- Market data provider trait (pluggable interface)
- Yahoo Finance provider implementation
- Price history storage and ingestion
- Net worth tracking with historical snapshots
- Reports: cash flow, trial balance, capital gains
- Export: CSV, JSON, hledger journal (one-way)

**Exit criteria**: A user can track investments with tax lots and see their net worth over time.

---

## Phase 3 — Market Tracker & Budgeting

**Goal**: Rich market data TUI and budgeting module.

### Deliverables

- Alpha Vantage and CoinGecko market data providers
- Market data caching layer with configurable TTL and rate limiting
- CLI TUI: real-time watchlist with sparklines (ratatui)
- CLI TUI: portfolio view (holdings, market value, gain/loss, allocation %)
- CLI TUI: interactive price charts (line, candlestick) with timeframe zoom
- Standalone market tracker mode (`ldgr watch` works without a vault)
- Budgeting module: envelope and zero-based methods
- Recurring transaction detection (rule-based pattern matching)
- Budget vs actual reporting
- Financial goals: basic goal tracking with linear/compound projections
- Commands: `ldgr budget`, `ldgr goals`, `ldgr networth`

**Exit criteria**: A user can monitor markets in real-time, set budgets, and track financial goals.

---

## Phase 4 — Sync & iPhone/iPad

**Goal**: Cross-device sync and native Apple apps.

### Deliverables

- Sync event generation from local mutations
- Event batch encryption/decryption
- Conflict detection via vector clocks
- Conflict resolution UI (CLI: interactive merge)
- Snapshot/compaction mechanism for efficient onboarding
- Blob store sync transport: Dropbox API
- WebDAV sync transport
- Device onboarding via QR code (X25519 key exchange)
- UniFFI bindings generation (XCFramework → Swift Package)
- Swift async wrapper layer over UniFFI
- iOS app: vault management (unlock, Face ID/Touch ID)
- iOS app: dashboard (net worth, recent transactions, budget summary)
- iOS app: transaction list, add, edit
- iOS app: accounts with register drill-down
- iOS app: investment portfolio view
- iOS app: budget overview
- iPad: multi-column layouts, sidebar navigation
- Background sync with offline-first architecture
- Sync conflict resolution UI (iOS)
- Commands: `ldgr sync`, `ldgr devices`

**Exit criteria**: A user can add a transaction on their phone and see it on their laptop after sync, with conflicts handled gracefully.

---

## Phase 5 — Web & Advanced Features

**Goal**: Web app and advanced financial tools.

### Deliverables

- WASM build pipeline (feature-flagged, < 2 MB compressed)
- WASM bundle size optimization and CI enforcement
- Next.js app shell with client-side vault operations
- Web: vault unlock and session management (WebCrypto)
- Web: dashboard, transactions, accounts, investments, budget, market watchlist
- Service worker for offline access
- ~~Self-hosted sync server (Axum, AGPL-3.0)~~ ✅
- ~~SRP-6a authentication for server sync~~ ✅
- Loan tracking module: amortization schedules, payoff projections, what-if analysis, refinance comparison
- Advanced financial goal projections
- PDF report generation

**Exit criteria**: A user can access finances from any browser with offline support, and optionally self-host a sync server.

---

## Phase 6 — Polish & Ecosystem

**Goal**: Apple Watch, widgets, and community ecosystem.

### Deliverables

- ✅ Apple Watch app: net worth glance, portfolio summary, budget remaining
- ✅ Watch complications: net worth, daily spend, portfolio gain/loss
- iOS Widgets (WidgetKit): net worth, budget remaining, portfolio value
- Siri Shortcuts (App Intents): quick transaction entry
- Community market data provider interface and documentation
- CLI and web theming system
- Plugin/extension architecture for community features

**Exit criteria**: Full ecosystem with all platforms shipping, community contributions flowing.
