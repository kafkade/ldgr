//! Known-answer **structural** test vectors for the ldgr sync wire format (v1).
//!
//! This suite is the machine-checked, cross-language counterpart to
//! `docs/security/sync-test-vectors.md`. It pins the canonical, pre-encryption
//! JSON of the sync wire types so the iOS (`UniFFI`) and web (WASM) clients can
//! validate that they emit byte-identical structures:
//!
//! - the canonical [`EventBatch`] envelope (`serialize_batch`),
//! - the per-entity [`AccountPayload`] / [`TransactionPayload`] carried inside
//!   each event (`payload::to_bytes`),
//! - the SRP [`RegisterRequest`] auth body (`--features sync`).
//!
//! ## Why structural, not raw-ciphertext, vectors
//!
//! The on-the-wire sync *blob* is `json(encrypt_item(vault_key, json(batch)))`,
//! and `encrypt_item` draws a fresh random item key **and** a fresh random nonce
//! on every call (see `crypto::encrypt_item`). The ciphertext is therefore NOT
//! byte-reproducible and cannot be a golden vector. The genuine cross-language
//! contract is the **decrypted/structural** layer: the exact JSON every client
//! seals and the exact JSON every client unseals. That layer IS deterministic —
//! the `VectorClock` is a `BTreeMap` (sorted keys) and struct field order is
//! fixed by the type definitions — so it is what we pin here. No encryption and
//! no `test-vectors` feature are involved, so this suite runs under the normal
//! `cargo test` gate.
//!
//! Regenerate the fixtures (after an intentional, reviewed format change) with:
//!
//! ```sh
//! LDGR_REGENERATE_VECTORS=1 cargo test -p ldgr-core --all-features --test sync_vectors
//! ```
//!
//! The fixtures are otherwise treated as golden files: any drift fails CI.

use std::fs;
use std::path::{Path, PathBuf};

use ldgr_core::sync::events::{EntityType, Operation, SyncEvent, VectorClock, create_batch};
use ldgr_core::sync::payload::{self, AccountPayload, PostingPayload, TransactionPayload};
use ldgr_core::sync::serialize_batch;

const FIXTURES_DIR: &str = "tests/fixtures/sync";

fn regenerating() -> bool {
    std::env::var_os("LDGR_REGENERATE_VECTORS").is_some()
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES_DIR)
}

/// Assert that `canonical` (the exact, compact bytes the client emits on the
/// wire) equals the committed golden fixture `name`, byte-for-byte. This is a
/// true known-answer vector: iOS (`UniFFI`) and web (WASM) clients emit the same
/// compact JSON — same field order, same `BTreeMap`-sorted clock keys — and can
/// assert against these identical bytes. Regenerates the fixture when
/// `LDGR_REGENERATE_VECTORS` is set.
fn check_vector(name: &str, canonical: &[u8]) {
    let path = fixtures_dir().join(name);

    if regenerating() {
        fs::create_dir_all(fixtures_dir()).expect("create fixtures dir");
        // Sanity: the bytes must be valid JSON (catches accidental corruption).
        let _: serde_json::Value =
            serde_json::from_slice(canonical).expect("canonical bytes are valid JSON");
        fs::write(&path, canonical).unwrap_or_else(|e| panic!("write {name}: {e}"));
        return;
    }

    let golden = fs::read(&path).unwrap_or_else(|e| {
        panic!("missing golden fixture {name}: {e}\nregenerate with LDGR_REGENERATE_VECTORS=1")
    });

    assert_eq!(
        canonical,
        golden.as_slice(),
        "sync wire vector '{name}' drifted from the golden fixture (byte mismatch); if this \
         is an intentional, reviewed format change, regenerate with LDGR_REGENERATE_VECTORS=1"
    );
}

// ── Canonical inputs (fixed so the vectors are deterministic) ────────────────

const DEVICE: &str = "11111111-1111-7111-8111-111111111111";
const ACCOUNT_ID: &str = "aaaaaaaa-0000-7000-8000-000000000001";
const TXN_ID: &str = "bbbbbbbb-0000-7000-8000-000000000002";
const CASH_POSTING_ID: &str = "cccccccc-0000-7000-8000-000000000003";
const FOOD_POSTING_ID: &str = "dddddddd-0000-7000-8000-000000000004";
const TS: &str = "2024-01-15T12:00:00Z";

fn account_payload() -> AccountPayload {
    AccountPayload {
        id: ACCOUNT_ID.into(),
        name: "Assets:Cash".into(),
        account_type: "asset".into(),
        commodity: Some("USD".into()),
        parent_id: None,
        note: Some("petty cash".into()),
        created_at: TS.into(),
        modified_at: TS.into(),
    }
}

fn transaction_payload() -> TransactionPayload {
    TransactionPayload {
        id: TXN_ID.into(),
        date: "2024-01-15".into(),
        status: "cleared".into(),
        code: Some("REF-42".into()),
        description: "Lunch".into(),
        comment: Some("with team".into()),
        created_at: TS.into(),
        modified_at: TS.into(),
        postings: vec![
            PostingPayload {
                id: CASH_POSTING_ID.into(),
                account_id: ACCOUNT_ID.into(),
                amount_quantity: Some("-10.00".into()),
                amount_commodity: Some("USD".into()),
                balance_assertion_quantity: None,
                balance_assertion_commodity: None,
                created_at: TS.into(),
                version: 1,
            },
            PostingPayload {
                id: FOOD_POSTING_ID.into(),
                account_id: "aaaaaaaa-0000-7000-8000-000000000005".into(),
                amount_quantity: Some("10.00".into()),
                amount_commodity: Some("USD".into()),
                balance_assertion_quantity: None,
                balance_assertion_commodity: None,
                created_at: TS.into(),
                version: 1,
            },
        ],
    }
}

fn sync_event(
    entity_type: EntityType,
    entity_id: &str,
    payload: Vec<u8>,
    lamport: u64,
) -> SyncEvent {
    SyncEvent {
        id: format!("event-{lamport:020}"),
        device_id: DEVICE.into(),
        lamport_clock: lamport,
        entity_type,
        entity_id: entity_id.into(),
        operation: Operation::Create,
        payload,
        version: 1,
        created_at: TS.into(),
    }
}

// ── Vectors ──────────────────────────────────────────────────────────────────

#[test]
fn account_payload_vector_is_golden() {
    let bytes = payload::to_bytes(&account_payload()).expect("serialize account payload");
    check_vector("account_payload_v1.json", &bytes);
}

#[test]
fn transaction_payload_vector_is_golden() {
    let bytes = payload::to_bytes(&transaction_payload()).expect("serialize transaction payload");
    check_vector("transaction_payload_v1.json", &bytes);
}

#[test]
fn event_batch_vector_is_golden() {
    let account_event = sync_event(
        EntityType::Account,
        ACCOUNT_ID,
        payload::to_bytes(&account_payload()).unwrap(),
        1,
    );
    let txn_event = sync_event(
        EntityType::Transaction,
        TXN_ID,
        payload::to_bytes(&transaction_payload()).unwrap(),
        2,
    );

    let mut clock = VectorClock::default();
    clock.clocks.insert(DEVICE.into(), 2);

    let batch = create_batch(DEVICE, vec![account_event, txn_event], &clock);
    let bytes = serialize_batch(&batch).expect("serialize batch");
    check_vector("event_batch_v1.json", &bytes);
}

#[cfg(feature = "sync")]
#[test]
fn register_request_vectors_are_golden() {
    use ldgr_core::sync::server::protocol::RegisterRequest;

    // Single-secret (legacy SRP): `auth_scheme` omitted from the wire.
    let one_secret = RegisterRequest {
        username: "alice".into(),
        salt: "00112233445566778899aabbccddeeff".into(),
        verifier: "0123456789abcdef".into(),
        auth_scheme: None,
        account_id: None,
    };
    let bytes = serde_json::to_vec(&one_secret).expect("serialize register request");
    check_vector("register_request_1secret_v1.json", &bytes);

    // Two-secret (2SKD, ADR-008): `auth_scheme` and client-generated
    // `account_id` present.
    let two_skd = RegisterRequest {
        username: "carol".into(),
        salt: "ffeeddccbbaa99887766554433221100".into(),
        verifier: "fedcba9876543210".into(),
        auth_scheme: Some("srp-2skd-v1".into()),
        account_id: Some("018f5a3c-0000-7000-8000-000000000001".into()),
    };
    let bytes = serde_json::to_vec(&two_skd).expect("serialize 2skd register request");
    check_vector("register_request_2skd_v1.json", &bytes);
}
