# ADR-004: Data Model — SQLite-Canonical with Event Sync Layer

**Status**: Accepted  
**Date**: 2026-05-03  
**Decision makers**: @kafkade  

## Context

The internal data model must support: fast queries for reporting, efficient sync across devices, audit history for financial accountability, and encryption at the item level. Three approaches were evaluated:

- **Option A — Normalized relational model**: SQLite tables as source of truth.
- **Option B — Journal AST**: Parsed hledger journal representation.
- **Option C — Event log (event sourcing)**: Append-only events as source of truth, with materialized views.

Full event sourcing (Option C) was initially considered but critiqued for excessive complexity relative to value: it requires event replay, snapshot infrastructure, event schema evolution, and materialized view rebuilds — all for benefits that can be achieved more simply.

## Decision

**SQLite-canonical with versioned rows for audit history, plus a thin event sync layer.** This is simpler than full event sourcing but still supports conflict-aware sync.

### Architecture

```
┌─────────────────────────────────────┐
│         SQLite Database             │
│                                     │
│  ┌─────────────────────────────┐    │
│  │  Canonical Tables           │    │
│  │  (accounts, transactions,   │    │
│  │   postings, lots, prices,   │    │
│  │   budgets, goals, loans)    │    │
│  └─────────────────────────────┘    │
│                                     │
│  ┌─────────────────────────────┐    │
│  │  Sync Outbox                │    │
│  │  (pending events to push)   │    │
│  └─────────────────────────────┘    │
│                                     │
│  ┌─────────────────────────────┐    │
│  │  Sync State                 │    │
│  │  (vector clock, last sync)  │    │
│  └─────────────────────────────┘    │
└─────────────────────────────────────┘
```

### How Sync Works

1. Local mutation → write to canonical tables + append event to sync outbox
2. On sync push: encrypt outbox events in batches → upload to blob store/server
3. On sync pull: download new batches → decrypt → apply to canonical tables (with conflict detection)
4. Conflicts: flag in `sync_conflicts` table → present to user

### Why Not Full Event Sourcing

- SQLite is the query layer AND the canonical store — no materialized view rebuilds
- Audit history via versioned rows (`version` column + soft deletes), not event replay
- Schema migrations are standard SQLite migrations, not event schema evolution across 20+ event types × multiple versions
- First sync uses snapshot (encrypted SQLite state), not full event log replay
- Accounting already has a natural audit trail (journal entries are immutable records of changes)

## Consequences

- Much simpler implementation than event sourcing
- SQLite is battle-tested on every target platform (iOS, WASM via sql.js, desktop)
- Sync layer is a thin wrapper over canonical data, not a foundational architectural pattern
- History is preserved but queried via simple SQL, not event replay
- Schema migrations follow well-understood patterns (ALTER TABLE, data migration scripts)
