use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use rand::Rng;
use serde::{Deserialize, Serialize};

use super::errors::CryptoError;
use super::keys::{ItemKey, MasterEncryptionKey, RecoveryKey, VaultKey};

/// Domain separation AAD tags.
const VAULT_WRAP_AAD: &[u8] = b"ldgr-vault-wrap-v1";
const ITEM_WRAP_AAD: &[u8] = b"ldgr-item-wrap-v1";
const RECOVERY_WRAP_AAD: &[u8] = b"ldgr-recovery-wrap-v1";

/// Current wrapping format version.
const WRAP_VERSION: u8 = 1;

/// Nonce size for AES-256-GCM (96 bits).
const NONCE_LEN: usize = 12;

/// A wrapped (encrypted) key with versioning for future-proofing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedKey {
    /// Format version for migration support.
    pub version: u8,
    /// Random nonce used for this wrapping operation.
    pub nonce: [u8; NONCE_LEN],
    /// The encrypted key material (ciphertext + AES-GCM auth tag).
    pub ciphertext: Vec<u8>,
}

/// Low-level: wrap a plaintext key with AES-256-GCM using the given wrapping key and AAD.
fn wrap_key_raw(
    wrapping_key: &[u8; 32],
    plaintext_key: &[u8; 32],
    aad: &[u8],
) -> Result<WrappedKey, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(wrapping_key)
        .map_err(|e| CryptoError::WrapFailed(e.to_string()))?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let payload = aes_gcm::aead::Payload {
        msg: plaintext_key,
        aad,
    };

    let ciphertext = cipher
        .encrypt(nonce, payload)
        .map_err(|_| CryptoError::WrapFailed("AES-GCM encryption failed".into()))?;

    Ok(WrappedKey {
        version: WRAP_VERSION,
        nonce: nonce_bytes,
        ciphertext,
    })
}

/// Low-level: unwrap an encrypted key with AES-256-GCM using the given wrapping key and AAD.
fn unwrap_key_raw(
    wrapping_key: &[u8; 32],
    wrapped: &WrappedKey,
    aad: &[u8],
) -> Result<[u8; 32], CryptoError> {
    if wrapped.version != WRAP_VERSION {
        return Err(CryptoError::InvalidParams(format!(
            "unsupported wrap version: {}",
            wrapped.version
        )));
    }

    let cipher = Aes256Gcm::new_from_slice(wrapping_key)
        .map_err(|e| CryptoError::WrapFailed(e.to_string()))?;
    let nonce = Nonce::from_slice(&wrapped.nonce);

    let payload = aes_gcm::aead::Payload {
        msg: &wrapped.ciphertext,
        aad,
    };

    let plaintext = cipher
        .decrypt(nonce, payload)
        .map_err(|_| CryptoError::UnwrapFailed)?;

    let key_bytes: [u8; 32] = plaintext
        .try_into()
        .map_err(|_| CryptoError::UnwrapFailed)?;

    Ok(key_bytes)
}

// --- Typed public API ---

/// Wrap a vault key with the master encryption key.
///
/// Uses AAD `"ldgr-vault-wrap-v1"` for domain separation.
///
/// # Errors
///
/// Returns `CryptoError::WrapFailed` if AES-GCM encryption fails.
pub fn wrap_vault_key(
    mek: &MasterEncryptionKey,
    vault_key: &VaultKey,
) -> Result<WrappedKey, CryptoError> {
    wrap_key_raw(mek.as_bytes(), vault_key.as_bytes(), VAULT_WRAP_AAD)
}

/// Unwrap a vault key using the master encryption key.
///
/// # Errors
///
/// Returns `CryptoError::UnwrapFailed` if decryption or authentication fails,
/// or `CryptoError::InvalidParams` if the wrap version is unsupported.
pub fn unwrap_vault_key(
    mek: &MasterEncryptionKey,
    wrapped: &WrappedKey,
) -> Result<VaultKey, CryptoError> {
    let bytes = unwrap_key_raw(mek.as_bytes(), wrapped, VAULT_WRAP_AAD)?;
    Ok(VaultKey::from_bytes(bytes))
}

/// Wrap a vault key with a recovery key for emergency access.
///
/// Uses AAD `"ldgr-recovery-wrap-v1"` for domain separation.
///
/// # Errors
///
/// Returns `CryptoError::WrapFailed` if AES-GCM encryption fails.
pub fn wrap_vault_key_with_recovery(
    recovery_key: &RecoveryKey,
    vault_key: &VaultKey,
) -> Result<WrappedKey, CryptoError> {
    wrap_key_raw(
        recovery_key.as_bytes(),
        vault_key.as_bytes(),
        RECOVERY_WRAP_AAD,
    )
}

/// Unwrap a vault key using a recovery key.
///
/// # Errors
///
/// Returns `CryptoError::UnwrapFailed` if decryption or authentication fails.
pub fn unwrap_vault_key_with_recovery(
    recovery_key: &RecoveryKey,
    wrapped: &WrappedKey,
) -> Result<VaultKey, CryptoError> {
    let bytes = unwrap_key_raw(recovery_key.as_bytes(), wrapped, RECOVERY_WRAP_AAD)?;
    Ok(VaultKey::from_bytes(bytes))
}

/// Wrap an item key with the vault key.
///
/// Uses AAD `"ldgr-item-wrap-v1"` for domain separation.
///
/// # Errors
///
/// Returns `CryptoError::WrapFailed` if AES-GCM encryption fails.
pub fn wrap_item_key(vault_key: &VaultKey, item_key: &ItemKey) -> Result<WrappedKey, CryptoError> {
    wrap_key_raw(vault_key.as_bytes(), item_key.as_bytes(), ITEM_WRAP_AAD)
}

/// Unwrap an item key using the vault key.
///
/// # Errors
///
/// Returns `CryptoError::UnwrapFailed` if decryption or authentication fails.
pub fn unwrap_item_key(vault_key: &VaultKey, wrapped: &WrappedKey) -> Result<ItemKey, CryptoError> {
    let bytes = unwrap_key_raw(vault_key.as_bytes(), wrapped, ITEM_WRAP_AAD)?;
    Ok(ItemKey::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::kdf::{Argon2Params, derive_encryption_key, derive_master_key};

    // --- Round-trip tests ---

    #[test]
    fn vault_key_wrap_unwrap_round_trip() {
        let mek = MasterEncryptionKey::from_bytes([0xAA; 32]);
        let vk = VaultKey::generate();

        let wrapped = wrap_vault_key(&mek, &vk).unwrap();
        let unwrapped = unwrap_vault_key(&mek, &wrapped).unwrap();
        assert_eq!(vk.as_bytes(), unwrapped.as_bytes());
    }

    #[test]
    fn item_key_wrap_unwrap_round_trip() {
        let vk = VaultKey::generate();
        let ik = ItemKey::generate();

        let wrapped = wrap_item_key(&vk, &ik).unwrap();
        let unwrapped = unwrap_item_key(&vk, &wrapped).unwrap();
        assert_eq!(ik.as_bytes(), unwrapped.as_bytes());
    }

    #[test]
    fn recovery_key_wrap_unwrap_round_trip() {
        let rk = RecoveryKey::generate();
        let vk = VaultKey::generate();

        let wrapped = wrap_vault_key_with_recovery(&rk, &vk).unwrap();
        let unwrapped = unwrap_vault_key_with_recovery(&rk, &wrapped).unwrap();
        assert_eq!(vk.as_bytes(), unwrapped.as_bytes());
    }

    // --- Failure tests ---

    #[test]
    fn wrong_mek_fails_vault_unwrap() {
        let mek1 = MasterEncryptionKey::from_bytes([0xAA; 32]);
        let mek2 = MasterEncryptionKey::from_bytes([0xBB; 32]);
        let vk = VaultKey::generate();

        let wrapped = wrap_vault_key(&mek1, &vk).unwrap();
        let result = unwrap_vault_key(&mek2, &wrapped);
        assert!(result.is_err(), "Wrong MEK must fail unwrap");
    }

    #[test]
    fn wrong_vault_key_fails_item_unwrap() {
        let vk1 = VaultKey::generate();
        let vk2 = VaultKey::generate();
        let ik = ItemKey::generate();

        let wrapped = wrap_item_key(&vk1, &ik).unwrap();
        let result = unwrap_item_key(&vk2, &wrapped);
        assert!(result.is_err(), "Wrong VK must fail item unwrap");
    }

    #[test]
    fn wrong_recovery_key_fails_unwrap() {
        let rk1 = RecoveryKey::generate();
        let rk2 = RecoveryKey::generate();
        let vk = VaultKey::generate();

        let wrapped = wrap_vault_key_with_recovery(&rk1, &vk).unwrap();
        let result = unwrap_vault_key_with_recovery(&rk2, &wrapped);
        assert!(result.is_err(), "Wrong recovery key must fail unwrap");
    }

    // --- Domain separation tests ---

    #[test]
    fn vault_wrapped_key_cannot_be_unwrapped_as_item() {
        // Use a key that works for both roles to isolate the AAD check
        let key_bytes = [0xCC; 32];
        let vk = VaultKey::from_bytes(key_bytes);
        let mek = MasterEncryptionKey::from_bytes(key_bytes);
        let target = VaultKey::generate();

        // Wrap as vault key (AAD = vault-wrap)
        let wrapped = wrap_vault_key(&mek, &target).unwrap();
        // Try to unwrap as item key (AAD = item-wrap) — must fail
        let result = unwrap_item_key(&vk, &wrapped);
        assert!(result.is_err(), "Cross-domain unwrap must fail");
    }

    #[test]
    fn recovery_wrapped_key_cannot_be_unwrapped_with_mek() {
        let key_bytes = [0xDD; 32];
        let rk = RecoveryKey::from_bytes(key_bytes);
        let mek = MasterEncryptionKey::from_bytes(key_bytes);
        let vk = VaultKey::generate();

        let wrapped = wrap_vault_key_with_recovery(&rk, &vk).unwrap();
        // Try to unwrap with MEK (wrong AAD) — must fail
        let result = unwrap_vault_key(&mek, &wrapped);
        assert!(
            result.is_err(),
            "Recovery wrap must not unwrap with vault AAD"
        );
    }

    // --- Corrupted data test ---

    #[test]
    fn corrupted_ciphertext_fails_unwrap() {
        let mek = MasterEncryptionKey::from_bytes([0xAA; 32]);
        let vk = VaultKey::generate();

        let mut wrapped = wrap_vault_key(&mek, &vk).unwrap();
        // Flip a byte in the ciphertext
        if let Some(byte) = wrapped.ciphertext.first_mut() {
            *byte ^= 0xFF;
        }
        let result = unwrap_vault_key(&mek, &wrapped);
        assert!(result.is_err(), "Corrupted ciphertext must fail");
    }

    #[test]
    fn corrupted_nonce_fails_unwrap() {
        let mek = MasterEncryptionKey::from_bytes([0xAA; 32]);
        let vk = VaultKey::generate();

        let mut wrapped = wrap_vault_key(&mek, &vk).unwrap();
        wrapped.nonce[0] ^= 0xFF;
        let result = unwrap_vault_key(&mek, &wrapped);
        assert!(result.is_err(), "Corrupted nonce must fail");
    }

    // --- End-to-end: password → vault key ---

    #[test]
    fn end_to_end_password_to_vault_key() {
        let params = Argon2Params::test();
        let salt = b"e2e-test-salt-16";
        let password = b"my-secret-password";

        // Derive key hierarchy
        let mk = derive_master_key(password, salt, &params).unwrap();
        let mek = derive_encryption_key(&mk).unwrap();

        // Generate and wrap vault key
        let vk = VaultKey::generate();
        let wrapped_vk = wrap_vault_key(&mek, &vk).unwrap();

        // Generate and wrap item key
        let ik = ItemKey::generate();
        let wrapped_ik = wrap_item_key(&vk, &ik).unwrap();

        // Re-derive from password and unwrap
        let mk2 = derive_master_key(password, salt, &params).unwrap();
        let mek2 = derive_encryption_key(&mk2).unwrap();
        let vk2 = unwrap_vault_key(&mek2, &wrapped_vk).unwrap();
        let ik2 = unwrap_item_key(&vk2, &wrapped_ik).unwrap();

        assert_eq!(vk.as_bytes(), vk2.as_bytes());
        assert_eq!(ik.as_bytes(), ik2.as_bytes());
    }

    #[test]
    fn end_to_end_wrong_password_fails() {
        let params = Argon2Params::test();
        let salt = b"e2e-test-salt-16";

        let mk = derive_master_key(b"correct-password", salt, &params).unwrap();
        let mek = derive_encryption_key(&mk).unwrap();
        let vk = VaultKey::generate();
        let wrapped_vk = wrap_vault_key(&mek, &vk).unwrap();

        // Wrong password
        let wrong_mk = derive_master_key(b"wrong-password", salt, &params).unwrap();
        let wrong_mek = derive_encryption_key(&wrong_mk).unwrap();
        let result = unwrap_vault_key(&wrong_mek, &wrapped_vk);
        assert!(result.is_err(), "Wrong password must fail to unlock vault");
    }

    // --- Password change flow ---

    #[test]
    fn password_change_preserves_vault_and_item_keys() {
        let params = Argon2Params::test();
        let salt = b"change-test-sa16";

        // Initial setup
        let mk_old = derive_master_key(b"old-password", salt, &params).unwrap();
        let mek_old = derive_encryption_key(&mk_old).unwrap();
        let vk = VaultKey::generate();
        let ik = ItemKey::generate();
        let wrapped_vk_old = wrap_vault_key(&mek_old, &vk).unwrap();
        let wrapped_ik = wrap_item_key(&vk, &ik).unwrap();

        // Password change: unwrap VK with old MEK, re-wrap with new MEK
        let unwrapped_vk = unwrap_vault_key(&mek_old, &wrapped_vk_old).unwrap();

        let mk_new = derive_master_key(b"new-password", salt, &params).unwrap();
        let mek_new = derive_encryption_key(&mk_new).unwrap();
        let wrapped_vk_new = wrap_vault_key(&mek_new, &unwrapped_vk).unwrap();

        // Verify: new password unlocks vault and items
        let vk_check = unwrap_vault_key(&mek_new, &wrapped_vk_new).unwrap();
        let ik_check = unwrap_item_key(&vk_check, &wrapped_ik).unwrap();

        assert_eq!(
            vk.as_bytes(),
            vk_check.as_bytes(),
            "VK must survive password change"
        );
        assert_eq!(
            ik.as_bytes(),
            ik_check.as_bytes(),
            "IK must survive password change"
        );

        // Old password no longer works
        let result = unwrap_vault_key(&mek_old, &wrapped_vk_new);
        assert!(
            result.is_err(),
            "Old password must not unlock re-wrapped VK"
        );
    }

    // --- Recovery flow ---

    #[test]
    fn recovery_key_unlocks_vault_after_password_loss() {
        let params = Argon2Params::test();
        let salt = b"recovery-test-16";

        // Setup: create vault with both MEK and recovery wrapping
        let mk = derive_master_key(b"forgotten-password", salt, &params).unwrap();
        let mek = derive_encryption_key(&mk).unwrap();
        let vk = VaultKey::generate();
        let rk = RecoveryKey::generate();

        let _wrapped_vk_mek = wrap_vault_key(&mek, &vk).unwrap();
        let wrapped_vk_recovery = wrap_vault_key_with_recovery(&rk, &vk).unwrap();

        let ik = ItemKey::generate();
        let wrapped_ik = wrap_item_key(&vk, &ik).unwrap();

        // Recovery: use recovery key to unwrap VK, then set new password
        let recovered_vk = unwrap_vault_key_with_recovery(&rk, &wrapped_vk_recovery).unwrap();
        assert_eq!(vk.as_bytes(), recovered_vk.as_bytes());

        // Items are still accessible
        let recovered_ik = unwrap_item_key(&recovered_vk, &wrapped_ik).unwrap();
        assert_eq!(ik.as_bytes(), recovered_ik.as_bytes());

        // Re-wrap with new password
        let new_mk = derive_master_key(b"new-password-after-recovery", salt, &params).unwrap();
        let new_mek = derive_encryption_key(&new_mk).unwrap();
        let new_wrapped_vk = wrap_vault_key(&new_mek, &recovered_vk).unwrap();

        let final_vk = unwrap_vault_key(&new_mek, &new_wrapped_vk).unwrap();
        assert_eq!(vk.as_bytes(), final_vk.as_bytes());
    }

    // --- WrappedKey serialization ---

    #[test]
    fn wrapped_key_serializes_to_json() {
        let mek = MasterEncryptionKey::from_bytes([0xAA; 32]);
        let vk = VaultKey::generate();
        let wrapped = wrap_vault_key(&mek, &vk).unwrap();

        let json = serde_json::to_string(&wrapped).unwrap();
        let deserialized: WrappedKey = serde_json::from_str(&json).unwrap();

        let unwrapped = unwrap_vault_key(&mek, &deserialized).unwrap();
        assert_eq!(vk.as_bytes(), unwrapped.as_bytes());
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn vault_key_wrap_unwrap_prop(_seed in any::<u64>()) {
                let mek = MasterEncryptionKey::from_bytes(rand::random());
                let vk = VaultKey::generate();
                let wrapped = wrap_vault_key(&mek, &vk).unwrap();
                let unwrapped = unwrap_vault_key(&mek, &wrapped).unwrap();
                prop_assert_eq!(vk.as_bytes(), unwrapped.as_bytes());
            }

            #[test]
            fn item_key_wrap_unwrap_prop(_seed in any::<u64>()) {
                let vk = VaultKey::generate();
                let ik = ItemKey::generate();
                let wrapped = wrap_item_key(&vk, &ik).unwrap();
                let unwrapped = unwrap_item_key(&vk, &wrapped).unwrap();
                prop_assert_eq!(ik.as_bytes(), unwrapped.as_bytes());
            }

            #[test]
            fn recovery_key_wrap_unwrap_prop(_seed in any::<u64>()) {
                let rk = RecoveryKey::generate();
                let vk = VaultKey::generate();
                let wrapped = wrap_vault_key_with_recovery(&rk, &vk).unwrap();
                let unwrapped = unwrap_vault_key_with_recovery(&rk, &wrapped).unwrap();
                prop_assert_eq!(vk.as_bytes(), unwrapped.as_bytes());
            }
        }
    }
}
