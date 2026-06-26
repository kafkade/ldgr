//! In-process integration test: a **two-secret (2SKD)** registration verifier
//! produced by `ldgr-core` is accepted by the real `ldgr-server` SRP-6a verify
//! logic (`SrpHandshakeStore`), and a wrong/missing Secret Key is rejected.
//!
//! This drives the server's actual handshake store (the same code path the
//! HTTP `/login` endpoints use) — no sockets, no re-implemented server math.

use std::time::Duration;

use num_bigint::BigUint;

use ldgr_core::crypto::{Argon2Params, SecretKey, derive_auth_key, derive_master_key};
use ldgr_core::sync::server::{ClientLogin, register_2skd_with_salt};
use ldgr_server::auth::srp::SrpHandshakeStore;

/// Derive the existing `MK_auth` (`AuthKey`) from a password, as the client does.
fn auth_key(password: &[u8]) -> ldgr_core::crypto::AuthKey {
    let mk = derive_master_key(password, b"argon-salt-16byte", &Argon2Params::test())
        .expect("derive master key");
    derive_auth_key(&mk).expect("derive auth key")
}

/// Run a full 2SKD handshake of the core client against the real server store.
/// Returns `true` iff the server accepts the proof and the client accepts the
/// server's `M2`.
fn handshake(
    username: &str,
    account_id: uuid::Uuid,
    reg_auth: &ldgr_core::crypto::AuthKey,
    reg_secret: &SecretKey,
    login_auth: &ldgr_core::crypto::AuthKey,
    login_secret: &SecretKey,
) -> bool {
    // Client registration → (salt, verifier). Fixed salt for determinism.
    let salt = vec![0x5Au8; 16];
    let reg = register_2skd_with_salt(&account_id, reg_auth, reg_secret, salt);

    // Client login init.
    let (login, a_pub) = ClientLogin::start_2skd(
        username,
        account_id,
        login_auth.clone(),
        login_secret.clone(),
    );

    // Real server store performs initiate / verify.
    let store = SrpHandshakeStore::new(Duration::from_mins(1));
    let b_pub = store
        .initiate(
            "hs-1".into(),
            username.into(),
            BigUint::from_bytes_be(&a_pub),
            reg.salt.clone(),
            BigUint::from_bytes_be(&reg.verifier),
        )
        .expect("server initiate");

    let session = login
        .finish(&reg.salt, &b_pub.to_bytes_be())
        .expect("client finish");

    match store.verify("hs-1", session.proof()) {
        Ok((m2, who)) => who == username && session.verify_server_proof(&m2),
        Err(_) => false,
    }
}

#[test]
fn two_skd_verifier_is_accepted_by_server() {
    let account_id = uuid::Uuid::from_bytes([0x11; 16]);
    let mk_auth = auth_key(b"correct horse battery staple");
    let secret_key = SecretKey::generate(account_id);

    assert!(
        handshake(
            "alice",
            account_id,
            &mk_auth,
            &secret_key,
            &mk_auth,
            &secret_key,
        ),
        "server must accept a 2SKD verifier when password + Secret Key match"
    );
}

#[test]
fn wrong_secret_key_is_rejected_by_server() {
    let account_id = uuid::Uuid::from_bytes([0x22; 16]);
    let mk_auth = auth_key(b"correct horse battery staple");
    let registered = SecretKey::generate(account_id);
    let attacker = SecretKey::generate(account_id); // correct password, wrong Secret Key

    assert!(
        !handshake(
            "bob",
            account_id,
            &mk_auth,
            &registered,
            &mk_auth,
            &attacker,
        ),
        "server must reject login when the Secret Key is wrong, even with the correct password"
    );
}

#[test]
fn wrong_password_is_rejected_by_server() {
    let account_id = uuid::Uuid::from_bytes([0x33; 16]);
    let secret_key = SecretKey::generate(account_id);

    assert!(
        !handshake(
            "carol",
            account_id,
            &auth_key(b"right-password"),
            &secret_key,
            &auth_key(b"wrong-password"),
            &secret_key,
        ),
        "server must reject login when the password is wrong, even with the correct Secret Key"
    );
}
