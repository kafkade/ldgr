//! Cross-tenant authorization regression suite (issue #298, ADR-011).
//!
//! Two separate accounts are registered against the **same** in-process server
//! (one shared [`RouterSender`], so one database and one router). Account A
//! creates a vault and stores blobs and a device in it; account B — a fully
//! authenticated but unrelated tenant — must be rejected on every vault-scoped
//! endpoint, and must not be able to claim, block, or observe A's vault.
//!
//! Rejection is `404 Not Found`, not `403 Forbidden`: the server refuses to
//! confirm that another tenant's vault id exists at all, so the identifier
//! namespace cannot be probed.
//!
//! Every assertion here is a *regression* guard. Authorization was already
//! enforced by `require_vault_access` before #298; what was missing was a test
//! that fails loudly if a future handler forgets to call it.

mod common;

use common::{RouterSender, client_on};
use ldgr_core::sync::server::{
    ListBatchesQuery, ListSnapshotsQuery, RawHttpSender, ServerSyncClient, ServerSyncError,
};

const A_BLOB: &[u8] = b"\x00\xfe alice ciphertext";
const B_BLOB: &[u8] = b"\x00\xfe mallory ciphertext";

const A_DEVICE: &str = "device-a";
const A_BATCH: &str = "batch-0001";
const A_SNAPSHOT: &str = "snapshot-0001";

/// Assert an operation was refused with `404`, the "this vault does not exist
/// for you" response. A `2xx` here means a cross-tenant read or write succeeded.
#[track_caller]
fn assert_denied<T: std::fmt::Debug>(label: &str, result: Result<T, ServerSyncError>) {
    match result {
        Err(ServerSyncError::Http { status: 404, .. }) => {}
        Err(other) => panic!("{label}: expected HTTP 404, got {other:?}"),
        Ok(value) => panic!("{label}: CROSS-TENANT ACCESS SUCCEEDED, returned {value:?}"),
    }
}

/// Register and log in `username` on the shared server.
async fn account<S: RawHttpSender>(
    client: &mut ServerSyncClient<S>,
    username: &str,
    password: &[u8],
) {
    client.register(username, password).await.expect("register");
    client.login(username, password).await.expect("login");
    assert!(client.is_authenticated(), "{username} should be logged in");
}

/// Boot one server, register account A with a populated vault and account B as
/// an unrelated tenant. Returns `(alice, mallory, alice_vault_id)`.
async fn two_tenants() -> (
    ServerSyncClient<RouterSender>,
    ServerSyncClient<RouterSender>,
    String,
) {
    let sender = RouterSender::new();

    let mut alice = client_on(&sender);
    account(&mut alice, "alice", b"alice-correct-horse").await;

    let vault = alice
        .create_vault(None)
        .await
        .expect("alice creates a vault")
        .id;

    alice
        .put_batch(&vault, A_DEVICE, A_BATCH, A_BLOB)
        .await
        .expect("alice pushes a batch");
    alice
        .put_snapshot(&vault, A_SNAPSHOT, A_BLOB)
        .await
        .expect("alice pushes a snapshot");
    alice
        .put_device(&vault, A_DEVICE, b"alice-encrypted-device-info")
        .await
        .expect("alice registers a device");

    let mut mallory = client_on(&sender);
    account(&mut mallory, "mallory", b"mallory-correct-horse").await;

    (alice, mallory, vault)
}

// ── Vault-scoped endpoints ──────────────────────────────────────────────────────

#[tokio::test]
async fn other_tenant_is_denied_on_every_batch_endpoint() {
    let (_alice, mallory, vault) = two_tenants().await;

    assert_denied(
        "GET batch",
        mallory.get_batch(&vault, A_DEVICE, A_BATCH).await,
    );
    assert_denied(
        "PUT batch",
        mallory
            .put_batch(&vault, A_DEVICE, "batch-injected", B_BLOB)
            .await,
    );
    assert_denied(
        "LIST batches",
        mallory
            .list_batches(&vault, &ListBatchesQuery::default())
            .await,
    );
}

#[tokio::test]
async fn other_tenant_is_denied_on_every_snapshot_endpoint() {
    let (_alice, mallory, vault) = two_tenants().await;

    assert_denied(
        "GET snapshot",
        mallory.get_snapshot(&vault, A_SNAPSHOT).await,
    );
    assert_denied(
        "PUT snapshot",
        mallory
            .put_snapshot(&vault, "snapshot-injected", B_BLOB)
            .await,
    );
    assert_denied(
        "LIST snapshots",
        mallory
            .list_snapshots(&vault, &ListSnapshotsQuery::default())
            .await,
    );
}

#[tokio::test]
async fn other_tenant_is_denied_on_every_device_endpoint() {
    let (_alice, mallory, vault) = two_tenants().await;

    assert_denied("LIST devices", mallory.list_devices(&vault).await);
    assert_denied(
        "PUT device",
        mallory.put_device(&vault, "device-injected", b"info").await,
    );
    assert_denied(
        "DELETE device",
        mallory.delete_device(&vault, A_DEVICE).await,
    );
}

#[tokio::test]
async fn other_tenant_cannot_see_the_vault_in_its_own_listing() {
    let (_alice, mallory, vault) = two_tenants().await;

    let listed = mallory.list_vaults().await.expect("list own vaults");
    assert!(
        !listed.iter().any(|v| v.id == vault),
        "another tenant's vault leaked into the listing: {listed:?}"
    );
}

/// Positive control: the denials above must come from *authorization*, not from
/// a broken fixture. The owner still reaches everything after B's attempts, and
/// none of B's writes landed.
#[tokio::test]
async fn owner_still_reaches_everything_after_the_denied_attempts() {
    let (alice, mallory, vault) = two_tenants().await;

    let _ = mallory
        .put_batch(&vault, A_DEVICE, "batch-injected", B_BLOB)
        .await;
    let _ = mallory.delete_device(&vault, A_DEVICE).await;

    assert_eq!(
        alice
            .get_batch(&vault, A_DEVICE, A_BATCH)
            .await
            .expect("owner reads its batch"),
        A_BLOB
    );
    assert_eq!(
        alice
            .get_snapshot(&vault, A_SNAPSHOT)
            .await
            .expect("owner reads its snapshot"),
        A_BLOB
    );

    let batches = alice
        .list_batches(&vault, &ListBatchesQuery::default())
        .await
        .expect("owner lists batches");
    assert_eq!(
        batches.entries.len(),
        1,
        "a rejected cross-tenant write still landed: {batches:?}"
    );

    let devices = alice
        .list_devices(&vault)
        .await
        .expect("owner lists devices");
    assert_eq!(
        devices.len(),
        1,
        "a rejected cross-tenant delete still took effect: {devices:?}"
    );
}

// ── Identifier squatting ────────────────────────────────────────────────────────

#[tokio::test]
async fn claiming_another_tenants_vault_id_yields_a_different_vault() {
    let (alice, mallory, vault) = two_tenants().await;

    // Mallory asks for Alice's identifier verbatim. The server must neither
    // hand it over nor fail: it mints Mallory a fresh identifier instead.
    let claimed = mallory
        .create_vault(Some(&vault))
        .await
        .expect("claiming a taken id must succeed with a substitute, not error")
        .id;

    assert_ne!(
        claimed, vault,
        "server handed a tenant an identifier already owned by another account"
    );

    // The substitute is a real, usable vault of Mallory's own.
    mallory
        .put_batch(&claimed, "device-m", "batch-m", B_BLOB)
        .await
        .expect("mallory can use the vault she was given");

    // And Alice's vault is still hers alone.
    assert_denied(
        "GET batch after squat attempt",
        mallory.get_batch(&vault, A_DEVICE, A_BATCH).await,
    );
    assert_eq!(
        alice
            .get_batch(&vault, A_DEVICE, A_BATCH)
            .await
            .expect("owner unaffected by the squat attempt"),
        A_BLOB
    );
}

/// The bug this issue exists to fix, from the victim's side: a second account
/// asking for an identifier that a *first* account already took must still end
/// up with a working vault. Before #298 the server returned a conflict, which
/// clients swallowed at setup and then failed on at every push.
#[tokio::test]
async fn a_taken_identifier_never_locks_a_tenant_out() {
    let sender = RouterSender::new();

    // Both accounts derive the same identifier — exactly what the old
    // hash-of-the-default-vault-path scheme produced for every user.
    let shared = "vault_5f2e1c0a9b8d7e6f";

    let mut first = client_on(&sender);
    account(&mut first, "first", b"first-correct-horse").await;
    let first_vault = first
        .create_vault(Some(shared))
        .await
        .expect("first claim")
        .id;
    assert_eq!(first_vault, shared, "an unclaimed id should be granted");

    let mut second = client_on(&sender);
    account(&mut second, "second", b"second-correct-horse").await;
    let second_vault = second
        .create_vault(Some(shared))
        .await
        .expect("second account must not be locked out")
        .id;

    assert_ne!(second_vault, first_vault);
    second
        .put_batch(&second_vault, "device-2", "batch-2", B_BLOB)
        .await
        .expect("second account can sync into its own vault");
    first
        .put_batch(&first_vault, "device-1", "batch-1", A_BLOB)
        .await
        .expect("first account is unaffected");
}

#[tokio::test]
async fn reclaiming_an_owned_identifier_is_idempotent() {
    let (alice, _mallory, vault) = two_tenants().await;

    // Re-running setup on an already-configured device must return the same
    // vault, not mint a second one and orphan the uploaded blobs.
    let again = alice
        .create_vault(Some(&vault))
        .await
        .expect("re-claim own vault")
        .id;
    assert_eq!(again, vault);

    let vaults = alice.list_vaults().await.expect("list own vaults");
    assert_eq!(
        vaults.len(),
        1,
        "re-claiming duplicated the vault: {vaults:?}"
    );
    assert_eq!(
        alice
            .get_batch(&vault, A_DEVICE, A_BATCH)
            .await
            .expect("blobs survive a re-claim"),
        A_BLOB
    );
}

#[tokio::test]
async fn minted_identifiers_are_random_and_unguessable() {
    let sender = RouterSender::new();
    let mut c = client_on(&sender);
    account(&mut c, "randy", b"randy-correct-horse").await;

    let mut ids = std::collections::HashSet::new();
    for _ in 0..16 {
        let id = c.create_vault(None).await.expect("mint a vault").id;
        assert!(
            ldgr_core::sync::is_random_vault_id(&id),
            "server minted a non-random identifier: {id}"
        );
        assert!(ids.insert(id), "server minted a duplicate identifier");
    }
}

#[tokio::test]
async fn path_unsafe_identifiers_are_substituted_never_stored() {
    let sender = RouterSender::new();
    let mut c = client_on(&sender);
    account(&mut c, "picky", b"picky-correct-horse").await;

    // A path separator would let a caller escape its own blob namespace. The
    // server never stores one — but it substitutes rather than failing, so an
    // older client that sends something odd still ends up with a usable vault.
    for bad in ["../etc/passwd", "vault/batches", "vault id", "vault.id"] {
        let granted = c
            .create_vault(Some(bad))
            .await
            .expect("an unusable identifier must be substituted, not rejected")
            .id;
        assert_ne!(granted, bad, "server stored a path-unsafe identifier");
        assert!(
            ldgr_core::sync::is_random_vault_id(&granted),
            "expected a minted substitute, got {granted}"
        );
    }

    let owned = c.list_vaults().await.expect("list own vaults");
    assert!(
        owned
            .iter()
            .all(|v| ldgr_core::sync::is_valid_vault_id(&v.id)),
        "a path-unsafe identifier reached the vaults table: {owned:?}"
    );
}

/// Length is still a hard contract, matching what pre-ADR-011 servers enforced.
#[tokio::test]
async fn empty_and_oversized_identifiers_are_rejected() {
    let sender = RouterSender::new();
    let mut c = client_on(&sender);
    account(&mut c, "lengthy", b"lengthy-correct-horse").await;

    for bad in [String::new(), "a".repeat(129)] {
        match c.create_vault(Some(&bad)).await {
            Err(ServerSyncError::Http { status: 400, .. }) => {}
            other => panic!(
                "expected 400 for a {}-char vault_id, got {other:?}",
                bad.len()
            ),
        }
    }
}

/// Older iOS and web builds let users type any vault identifier they liked, so
/// values like `Family Vault` exist on deployed servers. Tightening the
/// character set must not lock those accounts out of their own vault.
#[tokio::test]
async fn a_hand_typed_legacy_identifier_stays_claimable_by_its_owner() {
    let path = std::env::temp_dir().join(format!("ldgr-legacy-id-{}.db", uuid::Uuid::now_v7()));
    let path_str = path.to_str().unwrap().to_string();

    // Seed the pre-ADR-011 row directly — today's server would never mint it.
    {
        let conn = rusqlite::Connection::open(&path_str).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                 id TEXT PRIMARY KEY, username TEXT UNIQUE NOT NULL,
                 salt BLOB NOT NULL, verifier BLOB NOT NULL, created_at TEXT NOT NULL
             );
             CREATE TABLE vaults (
                 id TEXT PRIMARY KEY,
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 created_at TEXT NOT NULL
             );
             INSERT INTO users (id, username, salt, verifier, created_at)
             VALUES ('u1', 'vintage', x'5a', x'5a', '2020-01-01T00:00:00Z');
             INSERT INTO users (id, username, salt, verifier, created_at)
             VALUES ('u2', 'newcomer', x'5b', x'5b', '2020-01-02T00:00:00Z');
             INSERT INTO vaults (id, user_id, created_at)
             VALUES ('Family Vault', 'u1', '2020-01-01T00:00:00Z');",
        )
        .unwrap();
    }

    let db = ldgr_server::storage::ServerDb::open(&path_str).expect("open + migrate");
    let reclaimed = db
        .claim_vault(Some("Family Vault"), "u1")
        .await
        .expect("re-claim");
    assert_eq!(
        reclaimed.id, "Family Vault",
        "the owner was refused its own vault — sign-in would break for this account"
    );

    // A *different* account asking for the same identifier still gets a
    // substitute, and the substitute is always path-safe.
    let other = db
        .claim_vault(Some("Family Vault"), "u2")
        .await
        .expect("claim");
    assert_ne!(other.id, "Family Vault");
    assert!(ldgr_core::sync::is_random_vault_id(&other.id));

    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{path_str}-wal"));
    let _ = std::fs::remove_file(format!("{path_str}-shm"));
}

// ── Schema migration ────────────────────────────────────────────────────────────

/// A server that predates ADR-011 must pick up the tenant-scoped vault index in
/// place, without losing the vaults its users already registered.
#[tokio::test]
async fn legacy_database_gains_the_tenant_scoped_vault_index() {
    let path = std::env::temp_dir().join(format!("ldgr-vault-migrate-{}.db", uuid::Uuid::now_v7()));
    let path_str = path.to_str().unwrap().to_string();

    // Hand-build a pre-ADR-011 database: vaults keyed globally, no
    // (user_id, id) index, holding one already-registered path-derived vault.
    {
        let conn = rusqlite::Connection::open(&path_str).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                 id TEXT PRIMARY KEY, username TEXT UNIQUE NOT NULL,
                 salt BLOB NOT NULL, verifier BLOB NOT NULL, created_at TEXT NOT NULL
             );
             CREATE TABLE vaults (
                 id TEXT PRIMARY KEY,
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 created_at TEXT NOT NULL
             );
             INSERT INTO users (id, username, salt, verifier, created_at)
             VALUES ('u1', 'legacy', x'5a', x'5a', '2020-01-01T00:00:00Z');
             INSERT INTO vaults (id, user_id, created_at)
             VALUES ('vault_5f2e1c0a9b8d7e6f', 'u1', '2020-01-01T00:00:00Z');",
        )
        .unwrap();
    }

    let db = ldgr_server::storage::ServerDb::open(&path_str).expect("open + migrate");

    // The legacy vault survives and is still addressable by its old identifier.
    let vaults = db.list_user_vaults("u1").await.expect("list legacy vaults");
    assert_eq!(vaults.len(), 1);
    assert_eq!(vaults[0].id, "vault_5f2e1c0a9b8d7e6f");

    // Re-claiming it is idempotent, so an un-upgraded client keeps working.
    let reclaimed = db
        .claim_vault(Some("vault_5f2e1c0a9b8d7e6f"), "u1")
        .await
        .expect("legacy client re-claims its vault");
    assert_eq!(reclaimed.id, "vault_5f2e1c0a9b8d7e6f");

    {
        let conn = rusqlite::Connection::open(&path_str).unwrap();
        let index_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'index' AND name = 'idx_vaults_user_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_exists, 1, "tenant-scoped vault index was not created");
    }

    // Re-opening an already-migrated database must not fail.
    drop(db);
    let _ = ldgr_server::storage::ServerDb::open(&path_str).expect("re-open is idempotent");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{path_str}-wal"));
    let _ = std::fs::remove_file(format!("{path_str}-shm"));
}

// ── Relay (device pairing) ──────────────────────────────────────────────────────

/// Relay offers are keyed by an opaque offer id, so this locks in that they are
/// also bound to the account that created them — otherwise a third party who
/// learned an offer id could intercept a device-pairing exchange.
#[tokio::test]
async fn other_tenant_cannot_touch_a_relay_offer() {
    let (alice, mallory, _vault) = two_tenants().await;

    let offer = alice
        .create_offer(b"alice-encrypted-offer")
        .await
        .expect("alice opens a pairing offer");

    assert_denied("GET offer", mallory.get_offer(&offer.offer_id).await);
    assert_denied(
        "POST offer response",
        mallory
            .post_offer_response(&offer.offer_id, b"mallory-hijack")
            .await,
    );
    assert_denied(
        "GET offer response",
        mallory.get_offer_response(&offer.offer_id).await,
    );

    // The owner's exchange still completes untouched.
    alice
        .post_offer_response(&offer.offer_id, b"alice-encrypted-response")
        .await
        .expect("owner responds to its own offer");
    assert_eq!(
        alice
            .get_offer_response(&offer.offer_id)
            .await
            .expect("owner reads its own response"),
        b"alice-encrypted-response"
    );
}
