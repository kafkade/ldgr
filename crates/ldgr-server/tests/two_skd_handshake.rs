//! In-process integration test: a **two-secret (2SKD)** registration verifier
//! produced by `ldgr-core` is accepted by the real `ldgr-server` SRP-6a verify
//! logic (`SrpHandshakeStore`), and a wrong/missing Secret Key is rejected.
//!
//! This drives the server's actual handshake store (the same code path the
//! HTTP `/login` endpoints use) — no sockets, no re-implemented server math.

use std::time::Duration;

use num_bigint::BigUint;

use ldgr_core::crypto::{AccountKdf, Argon2Params, SecretKey, derive_account_auth_key};
use ldgr_core::sync::server::{ClientLogin, register_2skd_with_salt};
use ldgr_server::auth::srp::SrpHandshakeStore;

/// Fixed account KDF (salt + params) shared by registration and login so both
/// derive the same `MK_auth` deterministically (#296).
fn test_account_kdf() -> AccountKdf {
    AccountKdf::from_parts(b"argon-salt-16byte".to_vec(), Argon2Params::test())
}

/// Derive the existing `MK_auth` (`AuthKey`) from a password, as the client does
/// at registration.
fn auth_key(password: &[u8]) -> ldgr_core::crypto::AuthKey {
    derive_account_auth_key(password, &test_account_kdf()).expect("derive auth key")
}

/// Run a full 2SKD handshake of the core client against the real server store.
/// Returns `true` iff the server accepts the proof and the client accepts the
/// server's `M2`.
fn handshake(
    username: &str,
    account_id: uuid::Uuid,
    reg_password: &[u8],
    reg_secret: &SecretKey,
    login_password: &[u8],
    login_secret: &SecretKey,
) -> bool {
    // Client registration → (salt, verifier). Fixed salt for determinism.
    let salt = vec![0x5Au8; 16];
    let reg = register_2skd_with_salt(&account_id, &auth_key(reg_password), reg_secret, salt);

    // Client login init. `MK_auth` is derived at finish() from the account KDF.
    let (mut login, a_pub) =
        ClientLogin::start_2skd(username, login_password, login_secret.clone());
    // Server echoes the stored account_id + account KDF at `login/init`.
    login.set_account_id(account_id);
    login.set_account_kdf(test_account_kdf());

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
    let password = b"correct horse battery staple";
    let secret_key = SecretKey::generate(account_id);

    assert!(
        handshake(
            "alice",
            account_id,
            password,
            &secret_key,
            password,
            &secret_key,
        ),
        "server must accept a 2SKD verifier when password + Secret Key match"
    );
}

#[test]
fn wrong_secret_key_is_rejected_by_server() {
    let account_id = uuid::Uuid::from_bytes([0x22; 16]);
    let password = b"correct horse battery staple";
    let registered = SecretKey::generate(account_id);
    let attacker = SecretKey::generate(account_id); // correct password, wrong Secret Key

    assert!(
        !handshake(
            "bob",
            account_id,
            password,
            &registered,
            password,
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
            b"right-password",
            &secret_key,
            b"wrong-password",
            &secret_key,
        ),
        "server must reject login when the password is wrong, even with the correct Secret Key"
    );
}
