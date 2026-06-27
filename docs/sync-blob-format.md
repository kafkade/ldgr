# Sync Batch-Blob Format

Status: Stable (v1)
Owning crate: `ldgr-core` (`sync::pipeline`, `crypto::envelope`, `sync::events`, `sync::payload`)
Related: [ADR-003 (Sync & Conflict Resolution)](adr/003-sync-conflict-resolution.md),
[ADR-004 (Data Model)](adr/004-data-model.md),
[Vault Format Spec](security/vault-format-spec.md)

This document specifies the byte layout of the `.enc` **batch blobs** moved by the
sync transports (`{vault_id}/batches/{device_id}/{batch_id}.enc`). The blob is the
unit of cross-device sync: one device composes its pending outbox events into a
single encrypted blob, uploads it, and other devices download and apply it.

The format is defined **once, in `ldgr-core`**, so the CLI (reqwest), iOS/macOS
(UniFFI/`ldgr-ffi`), and web (`ldgr-wasm`) clients all produce and consume
byte-identical framing. There is no per-binding (re)implementation of the
encrypt/serialize logic.

## Framing

A batch blob is produced by `sync::pipeline::export_pending_batch` as:

```
blob_bytes = serde_json::to_vec(
    encrypt_item( vault_key, serialize_batch(batch) )
)
```

Unwrapping that from the inside out:

1. **`EventBatch`** (`sync::events::EventBatch`) — the plaintext payload:

   ```jsonc
   {
     "device_id": "…",            // originating device
     "events": [ SyncEvent, … ],  // ordered ascending by lamport_clock
     "vector_clock": { "clocks": { "<device_id>": <u64>, … } }
   }
   ```

   Each `SyncEvent` (`sync::events::SyncEvent`):

   ```jsonc
   {
     "id": "<uuidv7>",
     "device_id": "…",
     "lamport_clock": <u64>,
     "entity_type": "Transaction" | "Account" | "Price" | "Budget" | "Goal",
     "entity_id": "<uuid>",
     "operation": "Create" | "Update" | "Delete",
     "payload": [<u8>, …],        // see "Event payloads" below
     "version": <u32>,            // resulting entity row version
     "created_at": "<rfc3339>"
   }
   ```

2. **`serialize_batch(batch)`** (`sync::events::serialize_batch`) — UTF-8
   `serde_json` bytes of the `EventBatch`.

3. **`encrypt_item(vault_key, plaintext)`** (`crypto::envelope`) — per-item
   envelope encryption: a fresh random `ItemKey` encrypts the size-bucket-padded
   plaintext with AES-256-GCM (AAD `ldgr-item-seal-v1`), and the `ItemKey` is
   wrapped by the `VaultKey`. Produces a `SealedEnvelope`:

   ```jsonc
   {
     "version": 1,                // envelope format version
     "wrapped_ik": { … },         // item key wrapped by the vault key
     "nonce": [<12 bytes>],       // AES-GCM nonce for the payload
     "ciphertext": [<u8>, …]      // padded plaintext + GCM auth tag
   }
   ```

   Padding to size buckets (512 B / 2 KB / 8 KB / 32 KB, then 32 KB multiples)
   hides the exact batch size from the server/transport.

4. **`serde_json::to_vec(envelope)`** — the final blob bytes. This is exactly
   what `ServerTransport`/`WebDavTransport`/`DropboxTransport` store as the
   opaque `.enc` body.

`ingest_batch` is the exact inverse: `serde_json::from_slice` → `decrypt_item`
→ `deserialize_batch`.

### "Byte-compatible" — what it means here

Cross-client compatibility means the **framing/format** is identical and any
client holding the vault key can decrypt and parse any other client's blob. It
does **not** mean two encryptions of the same batch yield identical bytes: every
`encrypt_item` call draws a fresh random item key and nonce, so the `wrapped_ik`,
`nonce`, and `ciphertext` differ each time. The round-trip guarantee is at the
**decrypted-entity** level: applying a blob reproduces the originating entity
rows field-for-field (id, names, amounts, postings, timestamps, version, deleted).

## Event payloads (`sync::payload`)

The `payload` bytes inside each `SyncEvent` are canonical `serde_json` of the
structs in `sync::payload` — the single source of truth shared by the outbox
emitters (`*_with_sync` storage variants) and the apply path. Per ADR-003 events
are transaction-atomic and carry **full entity state**:

- `Create` / `Update` account → `AccountPayload`
  (`id, name, account_type, commodity, parent_id, note, created_at, modified_at`)
- `Create` / `Update` transaction → `TransactionPayload`
  (`id, date, status, code, description, comment, created_at, modified_at,
  postings[]`), each posting a `PostingPayload`
  (`id, account_id, amount_quantity, amount_commodity,
  balance_assertion_quantity, balance_assertion_commodity, created_at, version`).
  Posting list order is significant and reproduced as `posting_order`.
- `Delete` (any entity) → `DeletePayload` (`id`) — soft delete.

## Apply semantics (`ingest_batch`)

1. Decrypt + deserialize the blob.
2. Three-way merge against local pending (unsynced) outbox events via
   `sync::conflicts::merge_events`:
   - Events touching an entity with no local pending edit apply cleanly.
   - Events touching an entity that **also** has a local pending edit are
     **conflicts** → persisted to `sync_conflicts` for user review (no
     last-write-wins; ADR-003).
   - Events already covered by the local vector clock are skipped.
3. Apply each cleanly-merged event to the canonical tables (`storage::apply_*`)
   in deterministic total order (lamport → event id → device id):
   - Upsert by the **explicit remote `entity_id`** (never minting a new id).
   - **Staleness guard:** apply iff the entity is unknown locally **or** the
     event's `version` is strictly greater than the local row's version;
     equal/older is skipped as already-seen. A `Delete` for an unknown entity is
     a no-op.
   - Applied remote events are written to entity tables only — they **never**
     record a `sync_events` outbox row, so they do not echo back as new local
     events.
4. Advance and persist clocks: merge the batch's vector clock into the local one
   (`sync_state["sync:vector_clock"]`) and raise the Lamport clock to the highest
   observed remote value. The whole apply runs in one SQLite transaction.

**Idempotency.** Re-ingesting the same (or an older) blob is a no-op: after the
first ingest the persisted local vector clock dominates the batch's clock, so the
merge step skips every event. The per-entity version guard is a second line of
defense against out-of-order delivery.

## Versioning & compatibility

- Envelope `version` is currently `1` (`crypto::envelope::ENVELOPE_VERSION`).
  `decrypt_item` rejects unknown envelope versions.
- The `EventBatch` / `SyncEvent` / payload shapes are versioned implicitly by the
  enum/struct definitions; additive changes should preserve existing field names.
- Supported entity types today: **Account** and **Transaction**.
  `Price`/`Budget`/`Goal` events fail closed with `PipelineError::UnsupportedEntity`
  (they have no storage module yet — tracked by **#203**) rather than being
  silently dropped.
