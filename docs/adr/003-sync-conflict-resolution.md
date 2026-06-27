# ADR-003: Sync & Conflict Resolution — Transaction-Atomic Events

**Status**: Accepted  
**Date**: 2026-05-03  
**Decision makers**: @kafkade  

## Context

ldgr must support cross-device sync (CLI ↔ iPhone ↔ iPad ↔ Watch ↔ Web) while maintaining zero-knowledge guarantees and double-entry accounting invariants. Concurrent offline edits on multiple devices are the hard problem.

Initial design considered last-write-wins (LWW) on individual field-level events. Critique identified that LWW breaks double-entry atomicity: if one device edits a transaction's amount and another edits its category, LWW can produce a transaction where postings don't balance.

## Decision

**Transaction-atomic event log with three-way merge and mandatory user review for conflicts.**

### Sync Transport (Layered)

| Layer | Transport | Use Case |
|-------|-----------|----------|
| **Primary (MVP)** | User-provided blob store via API (Dropbox API, Google Drive API, S3-compatible, WebDAV) | Easy adoption, no server setup |
| **Secondary** | Self-hosted sync server (AGPL-3.0) | Lower latency, total ordering |
| **Backup only** | iCloud Drive / file system | Vault backup (single encrypted file), NOT event sync |

iCloud Drive is explicitly excluded from event sync due to unreliable file coordination APIs (delayed propagation, silent failures, concurrent write corruption).

### Event Model

Events are **transaction-atomic** — a single event captures the full state of an entity (all postings for a transaction). No partial updates.

```
Event {
  id: UUIDv7,              // time-ordered, globally unique
  device_id: DeviceID,     // originating device
  lamport_clock: u64,      // logical clock
  entity_type: EntityType, // Transaction | Account | Price | Budget | Goal | Loan
  entity_id: UUID,         // the entity being modified
  operation: Operation,    // Create | Update | Delete
  payload: EncryptedBlob,  // full entity state, encrypted with vault item key
  version: u32,            // schema version for this event type
}
```

### Conflict Resolution

1. **Detection**: Vector clock per device. On sync, compare vector clocks to identify divergence.
2. **Non-conflicting**: Events touching different entities merge automatically (deterministic total order on Lamport clock → UUIDv7 → device_id).
3. **Conflicting** (same entity modified on multiple devices):
   - DO NOT auto-resolve with LWW — double-entry transactions are atomic.
   - Flag conflicts for user review with both versions side-by-side.
   - Until resolved, newest version is displayed with a conflict indicator.
4. **Post-merge validation**: After every sync, validate all double-entry invariants. If validation fails, flag for review.

### Event Batching

Events encrypted in batches (per-sync-session or daily chunks), not individually. Reduces encryption overhead, file system pressure, and sync enumeration cost.

The concrete batch-blob byte layout (compose/encrypt/apply) is specified in [Sync Batch-Blob Format](../sync-blob-format.md), implemented by `ldgr-core`'s `sync::pipeline` (`export_pending_batch` / `ingest_batch`).

### Event Log Compaction

Every 1,000 events OR monthly (whichever first), create a snapshot of materialized state. New device sync: download latest snapshot + event batches since snapshot. Target: new device onboarding < 30 seconds for 10 years of data.

### Device Onboarding

1. Existing device generates ephemeral X25519 keypair
2. Displays QR code with public key + connection info
3. New device scans, establishes encrypted channel
4. Vault key transferred over encrypted channel
5. New device derives MEK from master password, unwraps vault key, syncs

## Consequences

- No silent data loss from concurrent edits
- Slightly more user friction on conflicts (manual review) — correct for financial data
- Batch encryption is less granular than per-event but dramatically more efficient
- Snapshot mechanism adds complexity but bounds sync time
- Blob store sync avoids server dependency for most users
