# Sync end-to-end & conflict test suite

This is the operator guide for the cross-client sync integration suite added for
issue #165. It explains what the suite proves, how to run each part, and which
gates must pass before the work is accepted.

## What the suite proves

The earlier in-process e2e tests
([`crates/ldgr-server/tests/server_sync_client_e2e.rs`](../crates/ldgr-server/tests/server_sync_client_e2e.rs))
boot the **real** Axum router in-process and prove **transport + auth**, but they
push **opaque bytes** — the server is a blind blob store, so they never exercise
the #201 compose/apply pipeline or ADR-003 conflict resolution.

This suite closes that gap by driving the **real**
`sync::pipeline::{export_pending_batch, ingest_batch}` pipeline *through* the
booted server, plus golden cross-language wire vectors:

| Layer | File | Proves |
| --- | --- | --- |
| Real pipeline through the server | [`crates/ldgr-server/tests/sync_pipeline_e2e.rs`](../crates/ldgr-server/tests/sync_pipeline_e2e.rs) | Cross-device propagation, conflict surfacing (no silent LWW), onboarding-to-consistency. |
| Cross-language wire vectors (Rust) | [`crates/ldgr-core/tests/sync_vectors.rs`](../crates/ldgr-core/tests/sync_vectors.rs) | The canonical client emits byte-exact structural JSON. |
| Cross-language wire vectors (web) | [`apps/web/test/sync-wasm.mjs`](../apps/web/test/sync-wasm.mjs) | The web client reproduces the same golden bytes and round-trips real `sealBatch`/`openBatch`. |
| Shared harness | [`crates/ldgr-server/tests/common/mod.rs`](../crates/ldgr-server/tests/common/mod.rs) | Single source of truth for the in-process `RouterSender` (avoids copy-paste drift). |

### The in-process server harness

`common::RouterSender` implements `sync::client::RawHttpSender` by calling
`tower::ServiceExt::oneshot` against the real `api::router(shared_state)`. It
holds a `state::SharedState` (Arc over an in-memory `storage::ServerDb::open(":memory:")`,
the SRP handshake store, and `Config`), so auth tokens and stored blobs persist
across calls within a test. There are **no sockets and no real HTTP client** —
the tests exercise the genuine routing, auth, and blob-store code paths
deterministically and fast.

### The four pipeline tests (`sync_pipeline_e2e.rs`)

1. **`real_cross_device_propagation_round_trip`** — Device A (a real vault +
   SQLite connection seeded with accounts and a balanced transaction via the
   `*_with_sync` storage fns) calls `export_pending_batch`, pushes the ciphertext
   over `RouterSender`; device B (a fresh connection sharing A's `&VaultKey`)
   lists/downloads and calls `ingest_batch`. Asserts `applied > 0` and that the
   accounts + transaction materialize identically in B. Then asserts the **reverse**
   direction (B → A).
2. **`session_key_seam_propagates_through_server`** — the same flow, but A drives
   the FFI/WASM seam: a real `UnlockedVault` + `export_session_key()` + the
   `*_with_session_key` pipeline variants. Documents why both the `&VaultKey` and
   raw-session-key paths exist.
3. **`concurrent_divergent_edits_surface_conflict_not_lww`** — A and B start from
   the same synced transaction `T`, then concurrently emit divergent `Update`
   events; each ingests the other's batch. Asserts `IngestOutcome.conflicts > 0`
   with `applied == 0` for the conflicting event, a persisted `StoredConflict`
   (distinct local/remote event ids, `resolved == false`) — i.e. **no silent
   last-write-wins** — and that the post-merge state still satisfies the
   double-entry invariant (the transaction's postings sum to zero per commodity).
4. **`fresh_device_onboards_to_consistency_via_replay`** — a late-joining device
   pulls and ingests every batch and ends materially consistent with A.

> **Scope flag — snapshot onboarding.** Real materialized-state snapshot
> compose/apply is **not yet wired** in the pipeline (only the `Snapshot` struct
> and planning helpers exist; no fn builds a snapshot payload from SQLite state or
> applies one into a fresh connection). Test 4 therefore implements
> onboarding-to-consistency honestly as **full event-batch replay**, with a code
> comment flagging that real-state `Snapshot` onboarding is a separate, not-yet-built
> capability that is intentionally out of scope here.

### Where the double-entry invariant is checked

ADR-003 §"Post-merge validation" mandates validating double-entry invariants
after every sync. There is no transaction-level `validate_balanced` in core (the
`validate*` fns are budget/vault/loan only), so the conflict test checks it
explicitly: it loads the transaction via `storage::transactions::get_transaction`
and asserts the per-commodity sum of posting `amount_quantity`
(`rust_decimal::Decimal`) equals zero — see `assert_transaction_balanced` /
`assert_balanced` in `sync_pipeline_e2e.rs`.

## Running the suite

```sh
# The pipeline / conflict / onboarding tests (integration tests in ldgr-server).
# Requires the `sqlite` dev-dependency feature, which the crate's Cargo.toml
# already enables for tests — no extra flags needed.
cargo test -p ldgr-server

# Just this file:
cargo test -p ldgr-server --test sync_pipeline_e2e

# The Rust cross-language wire vectors:
cargo test -p ldgr-core --all-features --test sync_vectors

# The web cross-language vectors (the golden block runs even without a built
# wasm pkg; the seal/open round-trip block skips gracefully until you build it):
node --test apps/web/test/sync-wasm.mjs
# For the full web round-trip, build the wasm package first:
#   cd apps/web && npm run build:wasm && npm test
```

The sync wire vectors and their regeneration command are documented separately in
[`security/sync-test-vectors.md`](./security/sync-test-vectors.md).

## CI / merge gates

The whole suite runs under the existing `cargo test --workspace --all-features`
job in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) — the merge gate
— because the tests live in already-scanned `crates/*/tests/` directories. **No
new CI job and no `kafkade/github-infra` `repo_ldgr.tf` change are required.** The
only manifest change is adding `sqlite` to ldgr-server's **dev-dependency**
ldgr-core features (and a `rust_decimal` dev-dependency); neither affects the
shipped server binary or the WASM bundle budget.

Gates to run locally before opening a PR:

```sh
cargo fmt --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features
cargo test -p ldgr-server
cargo build -p ldgr-core --no-default-features --features core,sync --target wasm32-unknown-unknown
```
