# ldgr Web App

Minimal scaffold for the ldgr web application. Uses ldgr-core compiled to WASM
for crypto and accounting, with sql.js for client-side SQLite storage.

## Architecture

```text
┌─────────────────────────────────┐
│       Browser (JS/TS)           │
│  ┌───────────┐  ┌────────────┐  │
│  │  ldgr-wasm│  │   sql.js   │  │
│  │  (crypto, │  │  (SQLite   │  │
│  │  parsing, │  │   in WASM) │  │
│  │  reports) │  │            │  │
│  └───────────┘  └────────────┘  │
│         │              │        │
│         └──────┬───────┘        │
│                │                │
│         IndexedDB / OPFS        │
└─────────────────────────────────┘
```

- **ldgr-wasm**: Rust core compiled to WASM via wasm-bindgen. Handles vault
  crypto (Argon2id key derivation, AES-256-GCM encryption), journal parsing,
  and balance/register report computation. No SQLite — pure computation only.

- **sql.js**: SQLite compiled to WASM via Emscripten. Provides the same schema
  as the native app for local persistence. Data is stored in-memory during use
  and persisted to IndexedDB or OPFS.

## SQLite-in-WASM Evaluation

| Library | Approach | Persistence | Status |
|---------|----------|-------------|--------|
| **sql.js** ✅ | Emscripten-compiled SQLite | IndexedDB export/import | Mature, well-maintained, chosen |
| wa-sqlite | WASM SQLite + VFS | OPFS (Origin Private File System) | Newer, good for large DBs |
| absurd-sql | sql.js + IndexedDB backend | IndexedDB pages | Unmaintained since 2022 |

**Decision**: sql.js is recommended for v1. It's battle-tested, has excellent
documentation, and the export/import-to-IndexedDB pattern is simple and reliable.
wa-sqlite with OPFS is a future optimization for large vaults (>10K transactions).

## Quick Start

```bash
# Build the WASM package
npm run build:wasm

# Install dependencies
npm install

# Run integration test
npm test
```

## Bundle Size Budget

Per ADR-005, the core WASM module must stay under **2 MB compressed**.
CI enforces this automatically on every PR.
