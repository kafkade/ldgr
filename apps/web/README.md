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

## Admin Panel

The web app includes an **admin panel** at `/admin` for managing a self-hosted
`ldgr-server` (users, invites, server settings, storage usage, and server info).

Per ADR-008 §7, the admin UI lives here in the Apache-2.0 web app and talks to
the headless AGPL server purely over its JSON API (`/api/v1/admin/*`); the server
serves no admin HTML/JS.

- **Sign-in** reuses the WASM SRP client, so the admin password is processed
  locally and only a proof is sent to the server. After the handshake the panel
  probes `GET /admin/stats`; a non-admin account is rejected, so only admins can
  reach the screens.
- **Adding users is invite-only** from the UI: issue an invite token and share it
  with the invitee, who redeems it during self-registration. (Direct
  password-based account creation would require exposing an SRP verifier helper
  from WASM and is intentionally out of scope.)
- The session token is held in memory and `sessionStorage` only — never written
  to a vault or `localStorage` — and is cleared on sign-out.

Because sign-in uses the SRP client, the admin panel needs the WASM bundle built
(`npm run build:wasm`), same as the rest of the app.
