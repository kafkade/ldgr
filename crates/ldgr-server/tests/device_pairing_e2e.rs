//! End-to-end device pairing test against the real `ldgr-server` relay.
//!
//! Drives the full two-offer X25519 handshake
//! ([`ldgr_core::sync::pairing`]) over the in-process axum router: device A
//! (`ldgr devices add`) initiates, device B (`ldgr devices join`) joins, and B
//! recovers the exact vault key held by A — with the relay only ever carrying
//! ciphertext (zero-knowledge).
//!
//! Both clients share one [`RouterSender`] so they hit the **same** server
//! state, and both authenticate as the **same account** (as two physical
//! devices of one user would), which is what makes the user-scoped relay offers
//! mutually visible.

mod common;

use common::RouterSender;

use ldgr_core::sync::VectorClock;
use ldgr_core::sync::pairing::{
    PairingCode, deliver_vault_key, initiate_pairing, poll_joiner_hello, poll_vault_key,
    respond_pairing,
};
use ldgr_core::sync::server::ServerSyncClient;
use ldgr_core::sync::transport::DeviceInfo;

const USER: &str = "pairing-user";
const PASSWORD: &[u8] = b"correct horse battery staple";

/// Register once with A, then log both A and B into the same account.
async fn two_devices() -> (
    ServerSyncClient<RouterSender>,
    ServerSyncClient<RouterSender>,
) {
    let sender = RouterSender::new();
    let mut a = ServerSyncClient::new(sender.clone());
    let mut b = ServerSyncClient::new(sender);

    a.register(USER, PASSWORD).await.expect("register");
    a.login(USER, PASSWORD).await.expect("login A");
    b.login(USER, PASSWORD).await.expect("login B");
    (a, b)
}

#[tokio::test]
async fn pairing_transfers_vault_key_over_encrypted_relay() {
    let (a, b) = two_devices().await;

    // The 32-byte vault/session key device A holds and must share with B.
    let vault_key: [u8; 32] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
        0xFF, 0x0F, 0x1E, 0x2D, 0x3C, 0x4B, 0x5A, 0x69, 0x78, 0x87, 0x96, 0xA5, 0xB4, 0xC3, 0xD2,
        0xE1, 0xF0,
    ];

    // ── Device A: `ldgr devices add` ────────────────────────────────────────
    let initiation = initiate_pairing(&a, "https://sync.example.com")
        .await
        .expect("initiate");
    let code = initiation.code.clone();
    assert_eq!(code.verification_code.len(), 6);
    assert_eq!(code.connection, "https://sync.example.com");

    // The out-of-band pairing token round-trips and only carries public fields.
    let token = code.encode();
    let decoded = PairingCode::decode(&token).expect("decode token");
    assert_eq!(decoded, code);

    // ── Device B: `ldgr devices join <token>` ───────────────────────────────
    let session = respond_pairing(&b, &decoded).await.expect("respond");
    // Both sides show the same MITM verification code.
    assert_eq!(session.verification_code, code.verification_code);

    // ── Device A observes B's hello and delivers the key ────────────────────
    // Before B responds there is nothing to read.
    assert!(
        poll_joiner_hello(&a, &code.offer_id)
            .await
            .expect("poll ok")
            .is_some(),
        "A should see B's hello after respond_pairing"
    );
    let hello = poll_joiner_hello(&a, &code.offer_id)
        .await
        .expect("poll ok")
        .expect("hello present");
    deliver_vault_key(&a, initiation, &hello, &vault_key)
        .await
        .expect("deliver");

    // ── Zero-knowledge: the relay wire carries only ciphertext ──────────────
    let on_the_wire = b
        .get_offer_response(session.response_offer_id())
        .await
        .expect("read raw relay response");
    assert_ne!(
        on_the_wire, vault_key,
        "the vault key must never appear in plaintext on the relay"
    );
    assert!(
        !on_the_wire.windows(vault_key.len()).any(|w| w == vault_key),
        "the plaintext vault key must not appear anywhere in the relay payload"
    );

    // ── Device B receives and unwraps the vault key ─────────────────────────
    let received = poll_vault_key(&b, &session)
        .await
        .expect("poll key")
        .expect("key present");
    assert_eq!(
        received, vault_key,
        "B must recover the exact vault key A holds — B can now unlock the vault"
    );
}

#[tokio::test]
async fn devices_list_register_and_remove_round_trip() {
    let (a, _b) = two_devices().await;

    let vault = "vault-pair";
    a.create_vault(vault).await.expect("create vault");

    // No devices registered yet.
    let devices = a.list_devices(vault).await.expect("list empty");
    assert!(devices.is_empty());

    // Register a device descriptor (as `sync push` / `devices join` would).
    let info = DeviceInfo {
        device_id: "device-xyz".into(),
        name: "Laptop".into(),
        platform: "cli".into(),
        last_sync_at: None,
        vector_clock: VectorClock::default(),
    };
    let bytes = serde_json::to_vec(&info).expect("serialize device info");
    a.put_device(vault, &info.device_id, &bytes)
        .await
        .expect("put device");

    let devices = a.list_devices(vault).await.expect("list one");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, "device-xyz");

    // Remove it.
    a.delete_device(vault, "device-xyz")
        .await
        .expect("delete device");
    let devices = a.list_devices(vault).await.expect("list empty again");
    assert!(devices.is_empty());
}
