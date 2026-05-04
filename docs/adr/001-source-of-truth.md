# ADR-001: Source of Truth — Vault-Canonical

**Status**: Accepted  
**Date**: 2026-05-03  
**Decision makers**: @kafkade  

## Context

There is a fundamental tension between plain-text accounting (human-readable journal files that tools like hledger operate on) and encrypted vault storage (a structured, encrypted database). ldgr must support hledger-compatible accounting while also providing zero-knowledge encryption across all platforms.

Three models were considered:

- **Model A — Vault-canonical**: The encrypted vault (internally a structured SQLite database) is the single source of truth. hledger journal format is the import/export interchange format only.
- **Model B — Journal-canonical**: The hledger journal file IS the source of truth. The vault encrypts the journal file(s). The app is essentially an encrypted wrapper around plain-text files.
- **Model C — Dual-mode**: The user chooses at vault creation time — "plain-text mode" for hledger purists or "vault mode" for consumers.

## Decision

**Model A — Vault-canonical.** The encrypted vault is the single source of truth. hledger journal format is an import/export interchange format, not the canonical store.

## Evaluation

| Criterion | Vault-Canonical (A) | Journal-Canonical (B) | Dual-Mode (C) |
|-----------|---------------------|----------------------|---------------|
| **Encryption granularity** | Per-item (transaction-level) | Per-file only | Mixed |
| **Sync/merge** | Tractable (structured items with UUIDs) | Git-merge territory (text diffs) | Two code paths |
| **Mobile/Watch performance** | SQLite queries are fast | Must parse journal text on every operation | Must support both |
| **hledger ecosystem** | Import/export only (lossy round-trip) | Full compatibility | Full in one mode |
| **Implementation complexity** | Single code path | Single code path | Double everything |

## Compatibility Boundary

- **Import**: Supports a documented strict subset of hledger syntax (see ADR-002). Unsupported features cause a clear error with guidance: _"This journal uses `include` directives. Flatten with `hledger print` first."_
- **Export**: One-way export to hledger journal format for reporting. Users pipe to hledger: `ldgr export --format hledger | hledger balance`. This is NOT bidirectional sync.
- **Validation tool**: `ldgr validate journal.hledger` checks importability before committing to migration.
- **Marketing**: ldgr is "hledger-compatible accounting" — not "hledger with encryption."

## Consequences

- Plain-text purists who want `vim` + `hledger` workflow won't adopt ldgr as their primary tool — and that's fine. We are targeting a different user: privacy-focused, multi-device, GUI-comfortable.
- Round-trip fidelity is lossy by design (include structure, comments, formatting are not preserved).
- The import contract is versioned and documented. Users know exactly what's supported before migrating.
- This decision enables per-transaction encryption, efficient mobile queries, and tractable sync — none of which are possible with journal-canonical.
