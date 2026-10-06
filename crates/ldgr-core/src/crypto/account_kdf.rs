//! Account-scoped Argon2id KDF material for **server auth** (ADR-008, #296).
//!
//! `MK_auth` (the SRP input) is derived with an Argon2id salt/params that are
//! **scoped to the account**, generated once at registration and decoupled from
//! any vault header. This is the fix for finding F2: previously `MK_auth` reused
//! the *vault's* Argon2 salt/params, so the same password + Secret Key produced a
//! different verifier per vault and a fresh device (no vault) could not reproduce
//! it.
//!
//! The account KDF is **not secret** — it is no more sensitive than the SRP salt
//! the server already returns pre-auth. It is stored server-side, returned at
//! `login/init`, and also carried in the Emergency Kit for offline/portable
//! onboarding. A substituted salt cannot leak anything: it only yields a wrong
//! (useless) `x`, so the SRP handshake simply fails.
//!
//! This type lives in `ldgr-core::crypto` (pure, WASM-safe) so every platform
//! derives `MK_auth` identically. It is intentionally available without the
//! `sync` feature because the Emergency Kit (core) carries a copy.

use rand::Rng;
use serde::{Deserialize, Serialize};

use super::errors::CryptoError;
use super::kdf::{Argon2Params, derive_auth_key, derive_master_key};
use super::keys::AuthKey;

/// Length of a freshly generated account KDF salt (bytes). Matches the SRP salt
/// length and comfortably exceeds Argon2's 8-byte minimum.
const ACCOUNT_KDF_SALT_LEN: usize = 16;

/// Account-scoped Argon2id salt + parameters used to derive `MK_auth`.
///
/// Generated once at account registration and pinned for the account's lifetime
/// (rotated only on an explicit credential change). Every device that signs in
/// must use these exact values — hence they are transported (server `login/init`
/// and the Emergency Kit) rather than recomputed from a device-local vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountKdf {
    /// Random per-account Argon2id salt (≥16 bytes).
    salt: Vec<u8>,
    /// Argon2id cost parameters chosen by the registering device and pinned.
    params: Argon2Params,
}

impl AccountKdf {
    /// Generate account KDF material with a fresh random salt and the given
    /// Argon2 parameters (typically the registering device's
    /// [`Argon2Params::desktop`]/[`mobile`](Argon2Params::mobile)/[`wasm`](Argon2Params::wasm)).
    #[must_use]
    pub fn generate(params: Argon2Params) -> Self {
        let mut salt = vec![0u8; ACCOUNT_KDF_SALT_LEN];
        rand::rng().fill_bytes(&mut salt);
        Self { salt, params }
    }

    /// Reconstruct account KDF material from a stored/transported salt + params
    /// (e.g. the server's `login/init` response or an Emergency Kit).
    #[must_use]
    pub fn from_parts(salt: Vec<u8>, params: Argon2Params) -> Self {
        Self { salt, params }
    }

    /// The account Argon2id salt.
    #[must_use]
    pub fn salt(&self) -> &[u8] {
        &self.salt
    }

    /// The account Argon2id parameters.
    #[must_use]
    pub fn params(&self) -> &Argon2Params {
        &self.params
    }
}

/// Derive the account **auth key** (`MK_auth`, the SRP input) from the master
/// `password` and the account-scoped [`AccountKdf`].
///
/// `MK_auth = HKDF(Argon2id(password, account_salt, account_params), "ldgr-auth-v1")`.
/// This is the *account-scoped* replacement for deriving `MK_auth` from a vault
/// header (ADR-008 Decision 1, #296).
///
/// # Errors
///
/// Returns [`CryptoError::InvalidParams`] if the Argon2 parameters are out of
/// range, or [`CryptoError::KeyDerivation`] if Argon2id/HKDF fails.
pub fn derive_account_auth_key(password: &[u8], kdf: &AccountKdf) -> Result<AuthKey, CryptoError> {
    let master_key = derive_master_key(password, kdf.salt(), kdf.params())?;
    derive_auth_key(&master_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_uses_full_length_random_salt() {
        let a = AccountKdf::generate(Argon2Params::test());
        let b = AccountKdf::generate(Argon2Params::test());
        assert_eq!(a.salt().len(), ACCOUNT_KDF_SALT_LEN);
        assert_ne!(a.salt(), b.salt(), "salts must be random per account");
    }

    #[test]
    fn from_parts_round_trips() {
        let kdf = AccountKdf::from_parts(vec![7u8; ACCOUNT_KDF_SALT_LEN], Argon2Params::test());
        assert_eq!(kdf.salt(), &[7u8; ACCOUNT_KDF_SALT_LEN]);
        assert_eq!(kdf.params().iterations, Argon2Params::test().iterations);
    }

    #[test]
    fn auth_key_is_deterministic_for_same_inputs() {
        let kdf = AccountKdf::from_parts(vec![1u8; ACCOUNT_KDF_SALT_LEN], Argon2Params::test());
        let a = derive_account_auth_key(b"password", &kdf).unwrap();
        let b = derive_account_auth_key(b"password", &kdf).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn auth_key_changes_with_salt() {
        let k1 = AccountKdf::from_parts(vec![1u8; ACCOUNT_KDF_SALT_LEN], Argon2Params::test());
        let k2 = AccountKdf::from_parts(vec![2u8; ACCOUNT_KDF_SALT_LEN], Argon2Params::test());
        let a = derive_account_auth_key(b"password", &k1).unwrap();
        let b = derive_account_auth_key(b"password", &k2).unwrap();
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn auth_key_changes_with_password() {
        let kdf = AccountKdf::from_parts(vec![1u8; ACCOUNT_KDF_SALT_LEN], Argon2Params::test());
        let a = derive_account_auth_key(b"password-1", &kdf).unwrap();
        let b = derive_account_auth_key(b"password-2", &kdf).unwrap();
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn serde_round_trip() {
        let kdf = AccountKdf::generate(Argon2Params::test());
        let json = serde_json::to_string(&kdf).unwrap();
        let back: AccountKdf = serde_json::from_str(&json).unwrap();
        assert_eq!(kdf, back);
    }
}
