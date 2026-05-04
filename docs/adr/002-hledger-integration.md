# ADR-002: hledger Integration — Rust-Native Parser (Strict Subset)

**Status**: Accepted  
**Date**: 2026-05-03  
**Decision makers**: @kafkade  

## Context

ldgr must provide accounting logic on all platforms: CLI (desktop), iOS/iPadOS/watchOS (via UniFFI), and web (via WASM). The hledger binary is a Haskell program that cannot be embedded on mobile or compiled to WASM.

Three options were evaluated:

- **Option A — Rust-native parser/engine**: Implement a compatible subset of hledger's journal format parser and reporting engine in Rust.
- **Option B — hledger binary wrapper**: Shell out to `hledger` for all accounting operations. Requires hledger installed on the host.
- **Option C — Embedded Haskell**: Embed hledger as a library (complex, likely impractical).

## Decision

**Option A — Rust-native parser for a strict, documented subset.** Use parser combinators (nom or winnow). Validate against hledger binary output in CI via differential testing.

## Supported Subset (v1.0)

| Feature | Supported | Notes |
|---------|-----------|-------|
| Plain transactions with postings | ✅ | Date, payee, accounts, amounts |
| Amount-less postings (auto-balance) | ✅ | Single posting per transaction |
| Balance assertions (single-commodity) | ✅ | |
| Account declarations | ✅ | |
| Commodity declarations | ✅ | |
| Price directives (`P`) | ✅ | |
| Tags and comments | ✅ | On transactions and postings |
| Transaction status (`*`, `!`) | ✅ | Cleared, pending |
| Multi-currency amounts | ✅ | |
| `include` directives | ❌ | Error: "Flatten with `hledger print` first" |
| Automated transactions (`=`) | ❌ | |
| Periodic transactions (`~`) | ❌ | |
| Multiple commodities in assertions | ❌ | |
| Lot notation (`{cost}`, `@@ total`) | ❌ | Phase 2: investment-specific syntax |
| Valuation expressions | ❌ | |
| Inline math expressions | ❌ | |
| Timedot format | ❌ | |

## Validation Strategy

1. **CI conformance tests**: Generate canonical test journals in the supported subset → parse with both ldgr and hledger → compare structured output.
2. **Differential fuzzing**: Property-based tests generating random valid journals in the supported subset → compare parse results.
3. **hledger test suite**: Run the subset of hledger's own tests that exercise supported features.
4. **Loud failure**: Any unsupported feature in an imported journal produces a clear, actionable error — never silent data loss.

## Desktop Hybrid Path (Optional)

On desktop where hledger binary is available, offer `ldgr import --via-hledger journal.hledger` which shells out to `hledger print --output-format=json` and imports the JSON. This handles edge cases the Rust parser doesn't yet cover, providing a migration path for complex journals.

## Consequences

- Scope is manageable (~3-4 months for core parser + reports, not 12+ for full parity)
- Mobile/WASM get full offline accounting without external dependencies
- Feature parity with hledger is a non-goal; ldgr is its own tool with hledger-compatible interchange
- The parser scope expands over time based on user demand
- The supported subset is versioned — users can check compatibility before migrating
