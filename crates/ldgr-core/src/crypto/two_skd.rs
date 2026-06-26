//! Two-secret key derivation (2SKD) — ADR-008, Decision 1.
//!
//! Derives the SRP-6a private-exponent seed (`x_seed`) from **both** the master
//! password (via the existing `MK_auth` / [`AuthKey`] path) and the account
//! [`SecretKey`]. Neither secret alone yields `x_seed`.
//!
//! ```text
//! SK_derived = HKDF-SHA256(ikm = SK_body, salt = account_id, info = "ldgr-secretkey-v1")
//! x_seed     = HKDF-SHA256(ikm = MK_auth || SK_derived,
//!                          salt = srp_salt,
//!                          info = "ldgr-2skd-v1" || 0x01 || account_id)
//! ```
//!
//! This module is pure (HKDF-SHA256 only — no big-integer arithmetic, no I/O),
//! so it compiles to WASM. Reducing `x_seed` mod N and computing the verifier
//! `v = g^x mod N` happens in the SRP client (`sync::server::srp`), behind the
//! `sync` feature.

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use super::errors::CryptoError;
use super::keys::AuthKey;
use super::secret_key::SecretKey;

/// HKDF info tag for `SK_derived`. Distinct from all vault tags.
const SECRET_KEY_INFO: &[u8] = b"ldgr-secretkey-v1";

/// HKDF info-tag prefix for `x_seed`. Distinct from all vault tags.
const TWO_SKD_INFO: &[u8] = b"ldgr-2skd-v1";

/// Construction version byte mixed into the `x_seed` HKDF info.
const TWO_SKD_VERSION: u8 = 0x01;

/// Length of every HKDF-SHA256 output used here.
const OUTPUT_LEN: usize = 32;

/// Derive the 32-byte SRP `x_seed` from the master auth key and the account
/// Secret Key, bound to `account_id` and the per-account `srp_salt`.
///
/// All intermediate key material (`SK_derived`, the 64-byte concatenation) is
/// zeroized; the returned seed is wrapped in [`Zeroizing`].
///
/// # Errors
///
/// Returns [`CryptoError::KeyDerivation`] if HKDF expansion fails (it cannot
/// for a 32-byte output, but the error is surfaced for completeness).
pub(crate) fn derive_x_seed(
    mk_auth: &AuthKey,
    secret_key: &SecretKey,
    account_id: &[u8],
    srp_salt: &[u8],
) -> Result<Zeroizing<[u8; OUTPUT_LEN]>, CryptoError> {
    // SK_derived = HKDF(ikm = SK_body, salt = account_id, info = "ldgr-secretkey-v1")
    let mut sk_derived = Zeroizing::new([0u8; OUTPUT_LEN]);
    Hkdf::<Sha256>::new(Some(account_id), secret_key.body())
        .expand(SECRET_KEY_INFO, sk_derived.as_mut_slice())
        .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;

    // ikm = MK_auth || SK_derived  (64 bytes)
    let mut ikm = Zeroizing::new([0u8; OUTPUT_LEN * 2]);
    ikm[..OUTPUT_LEN].copy_from_slice(mk_auth.as_bytes());
    ikm[OUTPUT_LEN..].copy_from_slice(sk_derived.as_slice());

    // info = "ldgr-2skd-v1" || 0x01 || account_id
    let mut info = Vec::with_capacity(TWO_SKD_INFO.len() + 1 + account_id.len());
    info.extend_from_slice(TWO_SKD_INFO);
    info.push(TWO_SKD_VERSION);
    info.extend_from_slice(account_id);

    let mut x_seed = Zeroizing::new([0u8; OUTPUT_LEN]);
    Hkdf::<Sha256>::new(Some(srp_salt), ikm.as_slice())
        .expand(&info, x_seed.as_mut_slice())
        .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;

    Ok(x_seed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Argon2Params, derive_auth_key, derive_master_key};
    use uuid::Uuid;

    fn auth_key(password: &[u8]) -> AuthKey {
        let mk = derive_master_key(password, b"argon-salt-16byte", &Argon2Params::test()).unwrap();
        derive_auth_key(&mk).unwrap()
    }

    fn account() -> Uuid {
        Uuid::from_bytes([7u8; 16])
    }

    #[test]
    fn deterministic_for_same_inputs() {
        let ak = auth_key(b"password");
        let sk = SecretKey::generate(account());
        let salt = b"srp-salt-16-bytes!";
        let a = derive_x_seed(&ak, &sk, account().as_bytes(), salt).unwrap();
        let b = derive_x_seed(&ak, &sk, account().as_bytes(), salt).unwrap();
        assert_eq!(a.as_slice(), b.as_slice());
    }

    #[test]
    fn different_secret_key_changes_seed() {
        let ak = auth_key(b"password");
        let sk1 = SecretKey::generate(account());
        let sk2 = SecretKey::generate(account());
        let salt = b"srp-salt-16-bytes!";
        let a = derive_x_seed(&ak, &sk1, account().as_bytes(), salt).unwrap();
        let b = derive_x_seed(&ak, &sk2, account().as_bytes(), salt).unwrap();
        assert_ne!(a.as_slice(), b.as_slice());
    }

    #[test]
    fn different_password_changes_seed() {
        let sk = SecretKey::generate(account());
        let salt = b"srp-salt-16-bytes!";
        let a = derive_x_seed(&auth_key(b"password-1"), &sk, account().as_bytes(), salt).unwrap();
        let b = derive_x_seed(&auth_key(b"password-2"), &sk, account().as_bytes(), salt).unwrap();
        assert_ne!(a.as_slice(), b.as_slice());
    }

    #[test]
    fn different_account_id_changes_seed() {
        let ak = auth_key(b"password");
        let sk = SecretKey::generate(account());
        let salt = b"srp-salt-16-bytes!";
        let a = derive_x_seed(&ak, &sk, &[1u8; 16], salt).unwrap();
        let b = derive_x_seed(&ak, &sk, &[2u8; 16], salt).unwrap();
        assert_ne!(a.as_slice(), b.as_slice());
    }

    #[test]
    fn different_salt_changes_seed() {
        let ak = auth_key(b"password");
        let sk = SecretKey::generate(account());
        let a = derive_x_seed(&ak, &sk, account().as_bytes(), b"salt-aaaaaaaaaaaa").unwrap();
        let b = derive_x_seed(&ak, &sk, account().as_bytes(), b"salt-bbbbbbbbbbbb").unwrap();
        assert_ne!(a.as_slice(), b.as_slice());
    }
}
