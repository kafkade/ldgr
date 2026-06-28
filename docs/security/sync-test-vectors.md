# ldgr sync wire format — test vectors (v1)

> **Status:** these vectors target the sync wire format **v1** — the
> pre-encryption structures exchanged by the #201 compose/apply pipeline and the
> SRP auth handshake. They are the canonical known-answer vectors for the iOS
> (UniFFI) and web (WASM) clients and for third-party implementations. The
> companion narrative spec for the encrypted blob is
> [`../sync-blob-format.md`](../sync-blob-format.md); the vault key hierarchy is in
> [`vault-format-spec.md`](./vault-format-spec.md). Where this document and the
> reference implementation disagree, the implementation wins.

## What these vectors are for

Every ldgr client (CLI, iOS, web) must emit **byte-identical** sync structures so
a batch sealed on one device decrypts and applies cleanly on another. These
vectors pin that contract. Each fixture is the exact compact JSON the reference
Rust client produces; a client is byte-compatible iff it reproduces every fixture
from the same inputs.

The reference implementation re-derives and checks all of them in CI via
[`crates/ldgr-core/tests/sync_vectors.rs`](../../crates/ldgr-core/tests/sync_vectors.rs),
and the web client checks the same fixtures from JavaScript in
[`apps/web/test/sync-wasm.mjs`](../../apps/web/test/sync-wasm.mjs) (the
`cross-language sync wire vectors (golden)` block).

## Why these are *structural*, not raw-ciphertext, vectors

The on-the-wire sync blob is:

```text
blob = json( encrypt_item( vault_key, json(EventBatch) ) )
```

`crypto::encrypt_item` draws a **fresh random item key and a fresh random nonce**
on every call, so the ciphertext is **not** byte-reproducible — it cannot be a
golden vector, by design (this is what makes the sync transport
indistinguishable-secure). The genuine, deterministic cross-language contract is
the layer *underneath* the encryption:

- the exact JSON each client **seals** (`EventBatch` + the per-entity payloads), and
- the exact JSON each client **unseals** and applies.

That layer is fully deterministic: the `VectorClock` is a `BTreeMap` (sorted
keys) and every struct serializes its fields in declaration order. No encryption
and no `test-vectors` cargo feature are involved, so this suite runs under the
normal `cargo test --workspace --all-features` gate (unlike the vault vectors,
which pin fixed nonces behind the `test-vectors` feature).

## Conventions

- All payloads are **compact** UTF-8 JSON (no spaces, no trailing newline) — the
  exact bytes that go over the wire.
- serde enums serialize as their **capitalized** variant names
  (`"Account"`, `"Transaction"`, `"Create"`, `"Update"`, `"Delete"`).
- An optional field marked `skip_serializing_if = "Option::is_none"` is **omitted
  entirely** when `None` (see `auth_scheme` in the single-secret register
  request) — it is *not* serialized as `null`.
- Inside an `EventBatch`, each event's `payload` is the inner payload JSON encoded
  as a **JSON array of bytes** (the UTF-8 bytes of that JSON). This matches the
  web outbox's `Array.from(new TextEncoder().encode(JSON.stringify(payload)))`.

## The vectors

All fixtures live under
[`crates/ldgr-core/tests/fixtures/sync/`](../../crates/ldgr-core/tests/fixtures/sync/),
described by `manifest.json` in that directory.

| Fixture | Type | Producer | What it pins |
| --- | --- | --- | --- |
| `account_payload_v1.json` | `sync::payload::AccountPayload` | `payload::to_bytes` | Account state carried in a Create/Update account event. |
| `transaction_payload_v1.json` | `sync::payload::TransactionPayload` | `payload::to_bytes` | Transaction + ordered postings carried in a transaction event. |
| `event_batch_v1.json` | `sync::events::EventBatch` | `serialize_batch` | The full pre-encryption batch envelope (device id, ordered events, vector clock). |
| `register_request_1secret_v1.json` | `sync::server::protocol::RegisterRequest` | `serde_json::to_vec` | Legacy single-secret SRP registration body (`auth_scheme` omitted). |
| `register_request_2skd_v1.json` | `sync::server::protocol::RegisterRequest` | `serde_json::to_vec` | Two-secret (2SKD, ADR-008) registration body (`auth_scheme: "srp-2skd-v1"`). |

### Canonical inputs

The inputs are fixed in `sync_vectors.rs` so the output is reproducible
(UUIDv7-shaped ids, a single fixed timestamp `2024-01-15T12:00:00Z`):

- **Account:** `Assets:Cash`, type `asset`, commodity `USD`, note `petty cash`.
- **Transaction:** date `2024-01-15`, status `cleared`, code `REF-42`,
  description `Lunch`, two balanced postings (`-10.00 USD` / `10.00 USD`).
- **Batch:** device `11111111-…`, two events (account create, transaction
  create), vector clock `{ "11111111-…": 2 }`.

### `account_payload_v1.json`

```json
{"id":"aaaaaaaa-0000-7000-8000-000000000001","name":"Assets:Cash","account_type":"asset","commodity":"USD","parent_id":null,"note":"petty cash","created_at":"2024-01-15T12:00:00Z","modified_at":"2024-01-15T12:00:00Z"}
```

### `register_request_1secret_v1.json`

```json
{"username":"alice","salt":"00112233445566778899aabbccddeeff","verifier":"0123456789abcdef"}
```

Note the absent `auth_scheme` — this keeps the body byte-identical to the
original (pre-2SKD) protocol.

### `register_request_2skd_v1.json`

```json
{"username":"carol","salt":"ffeeddccbbaa99887766554433221100","verifier":"fedcba9876543210","auth_scheme":"srp-2skd-v1"}
```

(`transaction_payload_v1.json` and `event_batch_v1.json` are larger; read them
directly from the fixtures directory.)

## Regenerating

The fixtures are golden files: any drift fails CI. After an **intentional,
reviewed** format change, regenerate them with:

```sh
LDGR_REGENERATE_VECTORS=1 cargo test -p ldgr-core --all-features --test sync_vectors
```

Then re-run the suite without the env var (and `node --test apps/web/test/sync-wasm.mjs`)
to confirm every client still matches, and commit the updated fixtures alongside
the format change.
