use argon2::{self, Algorithm, Argon2, Version};
use hkdf::Hkdf;
use sha2::Sha256;

use super::errors::CryptoError;
use super::keys::{AuthKey, MasterEncryptionKey, MasterKey};

/// HKDF info strings for domain separation.
const AUTH_KEY_INFO: &[u8] = b"ldgr-auth-v1";
const ENCRYPTION_KEY_INFO: &[u8] = b"ldgr-enc-v1";

/// Argon2id parameters for password hashing.
///
/// Stored in the vault header so the correct parameters are used on unlock.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Argon2Params {
    /// Memory cost in KiB.
    pub memory_cost_kib: u32,
    /// Number of iterations (time cost).
    pub iterations: u32,
    /// Degree of parallelism.
    pub parallelism: u32,
}

impl Argon2Params {
    /// Desktop defaults: 256 MB memory, 3 iterations, 4 threads.
    #[must_use]
    pub fn desktop() -> Self {
        Self {
            memory_cost_kib: 256 * 1024,
            iterations: 3,
            parallelism: 4,
        }
    }

    /// Mobile defaults: 64 MB memory, 4 iterations, 2 threads.
    #[must_use]
    pub fn mobile() -> Self {
        Self {
            memory_cost_kib: 64 * 1024,
            iterations: 4,
            parallelism: 2,
        }
    }

    /// Minimal parameters for testing. NOT suitable for production use.
    #[must_use]
    pub fn test() -> Self {
        Self {
            memory_cost_kib: 64,
            iterations: 1,
            parallelism: 1,
        }
    }

    fn validate(&self) -> Result<(), CryptoError> {
        if self.memory_cost_kib < 8 {
            return Err(CryptoError::InvalidParams(
                "memory_cost_kib must be at least 8 KiB".into(),
            ));
        }
        if self.iterations < 1 {
            return Err(CryptoError::InvalidParams(
                "iterations must be at least 1".into(),
            ));
        }
        if self.parallelism < 1 {
            return Err(CryptoError::InvalidParams(
                "parallelism must be at least 1".into(),
            ));
        }
        Ok(())
    }
}

/// Derive a master key from a password and salt using Argon2id.
///
/// The salt should be randomly generated and stored alongside the vault.
/// Returns a 256-bit master key.
///
/// # Errors
///
/// Returns `CryptoError::InvalidParams` if the parameters are out of range,
/// or `CryptoError::KeyDerivation` if Argon2id hashing fails.
pub fn derive_master_key(
    password: &[u8],
    salt: &[u8],
    params: &Argon2Params,
) -> Result<MasterKey, CryptoError> {
    params.validate()?;

    let argon2 = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        argon2::Params::new(
            params.memory_cost_kib,
            params.iterations,
            params.parallelism,
            Some(32),
        )
        .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?,
    );

    let mut output = [0u8; 32];
    argon2
        .hash_password_into(password, salt, &mut output)
        .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;

    Ok(MasterKey::from_bytes(output))
}

/// Derive an authentication key from the master key via HKDF-SHA256.
///
/// Uses the info string `"ldgr-auth-v1"` for domain separation.
///
/// # Errors
///
/// Returns `CryptoError::KeyDerivation` if HKDF expansion fails.
pub fn derive_auth_key(master_key: &MasterKey) -> Result<AuthKey, CryptoError> {
    let hk = Hkdf::<Sha256>::new(None, master_key.as_bytes());
    let mut output = [0u8; 32];
    hk.expand(AUTH_KEY_INFO, &mut output)
        .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;
    Ok(AuthKey::from_bytes(output))
}

/// Derive the master encryption key from the master key via HKDF-SHA256.
///
/// Uses the info string `"ldgr-enc-v1"` for domain separation.
///
/// # Errors
///
/// Returns `CryptoError::KeyDerivation` if HKDF expansion fails.
pub fn derive_encryption_key(master_key: &MasterKey) -> Result<MasterEncryptionKey, CryptoError> {
    let hk = Hkdf::<Sha256>::new(None, master_key.as_bytes());
    let mut output = [0u8; 32];
    hk.expand(ENCRYPTION_KEY_INFO, &mut output)
        .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;
    Ok(MasterEncryptionKey::from_bytes(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_params() -> Argon2Params {
        Argon2Params::test()
    }

    #[test]
    fn derive_master_key_deterministic() {
        let params = test_params();
        let salt = b"test-salt-16byte";
        let password = b"correct horse battery staple";

        let mk1 = derive_master_key(password, salt, &params).unwrap();
        let mk2 = derive_master_key(password, salt, &params).unwrap();
        assert_eq!(
            mk1.as_bytes(),
            mk2.as_bytes(),
            "Same inputs must produce same key"
        );
    }

    #[test]
    fn different_passwords_produce_different_keys() {
        let params = test_params();
        let salt = b"test-salt-16byte";

        let mk1 = derive_master_key(b"password-one", salt, &params).unwrap();
        let mk2 = derive_master_key(b"password-two", salt, &params).unwrap();
        assert_ne!(mk1.as_bytes(), mk2.as_bytes());
    }

    #[test]
    fn different_salts_produce_different_keys() {
        let params = test_params();
        let password = b"same-password";

        let mk1 = derive_master_key(password, b"salt-aaaaaaaaaa16", &params).unwrap();
        let mk2 = derive_master_key(password, b"salt-bbbbbbbbbb16", &params).unwrap();
        assert_ne!(mk1.as_bytes(), mk2.as_bytes());
    }

    #[test]
    fn auth_key_and_encryption_key_are_different() {
        let params = test_params();
        let mk = derive_master_key(b"password", b"salt-1234567890ab", &params).unwrap();

        let auth = derive_auth_key(&mk).unwrap();
        let enc = derive_encryption_key(&mk).unwrap();
        assert_ne!(
            auth.as_bytes(),
            enc.as_bytes(),
            "Auth and encryption keys must differ (domain separation)"
        );
    }

    #[test]
    fn derivation_is_deterministic_end_to_end() {
        let params = test_params();
        let mk = derive_master_key(b"password", b"salt-1234567890ab", &params).unwrap();

        let auth1 = derive_auth_key(&mk).unwrap();
        let auth2 = derive_auth_key(&mk).unwrap();
        assert_eq!(auth1.as_bytes(), auth2.as_bytes());

        let enc1 = derive_encryption_key(&mk).unwrap();
        let enc2 = derive_encryption_key(&mk).unwrap();
        assert_eq!(enc1.as_bytes(), enc2.as_bytes());
    }

    #[test]
    fn invalid_params_rejected() {
        let bad_params = Argon2Params {
            memory_cost_kib: 0,
            iterations: 1,
            parallelism: 1,
        };
        let result = derive_master_key(b"password", b"salt-1234567890ab", &bad_params);
        assert!(result.is_err());
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn derive_is_deterministic(
                password in proptest::collection::vec(any::<u8>(), 1..64),
                salt in proptest::collection::vec(any::<u8>(), 8..32),
            ) {
                let params = test_params();
                let mk1 = derive_master_key(&password, &salt, &params).unwrap();
                let mk2 = derive_master_key(&password, &salt, &params).unwrap();
                prop_assert_eq!(mk1.as_bytes(), mk2.as_bytes());
            }

            #[test]
            fn auth_and_enc_keys_always_differ(
                password in proptest::collection::vec(any::<u8>(), 1..64),
                salt in proptest::collection::vec(any::<u8>(), 8..32),
            ) {
                let params = test_params();
                let mk = derive_master_key(&password, &salt, &params).unwrap();
                let auth = derive_auth_key(&mk).unwrap();
                let enc = derive_encryption_key(&mk).unwrap();
                prop_assert_ne!(auth.as_bytes(), enc.as_bytes());
            }
        }
    }
}
