# ADR-005: Platform Boundaries — Maximalist Rust Core with Platform-Native Networking

**Status**: Accepted  
**Date**: 2026-05-03  
**Decision makers**: @kafkade  

## Context

ldgr targets four platform families: CLI (desktop), iOS/iPadOS/watchOS, web (browser), and an optional sync server. The tech stack is Rust core + SwiftUI shell + Next.js/TypeScript web. The question is where to draw the line between shared Rust code and platform-specific code.

## Decision

**Maximalist Rust core for all computation.** Platform-native code handles networking (HTTP), UI rendering, platform APIs (Keychain, biometrics), and chart rendering.

### Rust Core (`ldgr-core` crate)

| Module | Responsibility |
|--------|---------------|
| `crypto` | Key hierarchy, vault encryption/decryption, Argon2id, SRP-6a |
| `accounting` | Journal parser, double-entry validation, reports, queries |
| `storage` | SQLite schema, CRUD operations, versioned history |
| `sync` | Event generation, conflict detection, merge logic, snapshot creation |
| `import` | CSV, OFX/QFX, hledger parsing, deduplication, rules engine |
| `export` | hledger journal, CSV, JSON output generation |
| `market` | Quote data processing, caching, price history (NOT HTTP fetching) |
| `loans` | Amortization, payoff projections, what-if calculations |
| `budget` | Budget engine, allocation, recurring transaction detection |
| `goals` | Goal tracking, timeline projections |

### Platform-Specific Code

| Platform | Responsibility |
|----------|---------------|
| **CLI** (Rust) | clap commands, ratatui TUI, HTTP via reqwest |
| **iOS/iPadOS** (Swift) | SwiftUI views, URLSession for HTTP, Keychain/Secure Enclave, Face ID, Widgets, Shortcuts |
| **watchOS** (Swift) | SwiftUI complications, WatchConnectivity for companion data |
| **Web** (TypeScript) | Next.js shell, fetch API for HTTP, WebCrypto, service worker, IndexedDB |

### Cross-Platform Exposure

| Target | Mechanism | Notes |
|--------|-----------|-------|
| CLI | Native Rust binary | Full core, no FFI overhead |
| iOS/iPadOS | UniFFI → XCFramework → Swift Package | Swift async wrapper for long-running ops |
| watchOS | UniFFI → minimal subset | Decrypt + read-only queries only |
| Web | wasm-bindgen → npm package | Feature-flagged for bundle size control |

### WASM Bundle Strategy

Hard budget: **2 MB compressed** for initial load.

Feature flags in Cargo.toml control what's included:
- `core` (crypto + accounting + storage): ~1.5 MB — always loaded
- `sync`, `import-export`, `market`, `loans`, `budget`, `goals`: lazy-loaded modules

CI enforces the budget: build fails if `core` WASM exceeds 2 MB compressed.

### Market Data: Platform-Native Fetch, Rust Processing

```
Platform (Swift/TS): HTTP fetch → raw bytes
    ↓
Rust core: parse(bytes) → QuoteData → store in vault
    ↓
Platform (Swift/TS): render(QuoteData) → native charts
```

This avoids bundling reqwest in WASM and leverages platform networking (URLSession caching, background fetch, energy budgeting, system proxy).

### UniFFI Async Pattern

UniFFI generates callback-based APIs. A Swift wrapper layer provides idiomatic async/await:

```swift
// Swift wrapper provides async/await over UniFFI callbacks
public func sync() async throws -> SyncResult {
    try await withCheckedThrowingContinuation { continuation in
        syncVault { result in continuation.resume(with: result) }
    }
}
```

## Consequences

- Rust core is a pure computation library — no I/O, no networking, no platform dependencies
- WASM bundle stays small via feature flags and lazy loading
- Platform-native networking gets caching, proxy, and energy management for free
- UniFFI wrapper adds a thin Swift layer but provides idiomatic APIs
- The "no I/O" constraint makes the Rust core highly testable (pure functions, deterministic)
