//! Per-item envelope encryption with size-bucket padding.
//!
//! Each item gets a random [`ItemKey`] encrypted with the [`VaultKey`].
//! The plaintext payload is padded to a size bucket before encryption
//! to prevent leaking exact payload lengths.
//!
//! Padding uses a 4-byte big-endian length prefix followed by zero padding
//! to the nearest bucket boundary.

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use super::errors::CryptoError;
use super::keys::{ItemKey, VaultKey};
use super::wrap::{WrappedKey, unwrap_item_key, wrap_item_key};

/// AAD for item data encryption (distinct from key wrapping AAD).
const ITEM_SEAL_AAD: &[u8] = b"ldgr-item-seal-v1";

/// Current envelope format version.
const ENVELOPE_VERSION: u8 = 1;

/// Nonce size for AES-256-GCM (96 bits).
const NONCE_LEN: usize = 12;

/// Length-prefix size (u32 big-endian).
const LENGTH_PREFIX_LEN: usize = 4;

/// Size buckets in bytes.
const BUCKETS: &[usize] = &[512, 2_048, 8_192, 32_768];

/// Largest fixed bucket.
const LARGEST_BUCKET: usize = 32_768;

/// A sealed envelope containing an encrypted item payload.
///
/// The item key is wrapped with the vault key, and the payload is encrypted
/// with the item key. Both operations use AES-256-GCM with distinct AAD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedEnvelope {
    /// Format version for migration support.
    pub version: u8,
    /// The item key, wrapped (encrypted) by the vault key.
    pub wrapped_ik: WrappedKey,
    /// Random nonce used for payload encryption.
    pub nonce: [u8; NONCE_LEN],
    /// The encrypted (and padded) payload with AES-GCM auth tag appended.
    pub ciphertext: Vec<u8>,
}

/// Compute the padded size for a payload (including 4-byte length prefix).
///
/// Bucket boundaries:
/// - <= 512 B → 512 B
/// - <= 2 KB → 2 KB
/// - <= 8 KB → 8 KB
/// - <= 32 KB → 32 KB
/// - > 32 KB → nearest 32 KB multiple
fn padded_size(payload_len: usize) -> usize {
    let total = LENGTH_PREFIX_LEN + payload_len;
    for &bucket in BUCKETS {
        if total <= bucket {
            return bucket;
        }
    }
    // Round up to nearest LARGEST_BUCKET multiple
    total.div_ceil(LARGEST_BUCKET) * LARGEST_BUCKET
}

/// Pad plaintext with a 4-byte big-endian length prefix, then zero-pad to bucket.
fn pad_to_bucket(plaintext: &[u8]) -> Vec<u8> {
    let target_size = padded_size(plaintext.len());
    let mut padded = Vec::with_capacity(target_size);

    let len_bytes = u32::try_from(plaintext.len())
        .expect("plaintext must be < 4 GiB")
        .to_be_bytes();
    padded.extend_from_slice(&len_bytes);
    padded.extend_from_slice(plaintext);
    padded.resize(target_size, 0);

    padded
}

/// Remove length-prefix padding, returning the original plaintext.
fn unpad(padded: &[u8]) -> Result<&[u8], CryptoError> {
    if padded.len() < LENGTH_PREFIX_LEN {
        return Err(CryptoError::DecryptionFailed(
            "padded data too short for length prefix".into(),
        ));
    }

    let len = u32::from_be_bytes([padded[0], padded[1], padded[2], padded[3]]) as usize;

    if LENGTH_PREFIX_LEN + len > padded.len() {
        return Err(CryptoError::DecryptionFailed(
            "length prefix exceeds padded data".into(),
        ));
    }

    Ok(&padded[LENGTH_PREFIX_LEN..LENGTH_PREFIX_LEN + len])
}

/// Encrypt a plaintext payload using per-item envelope encryption.
///
/// 1. Generates a random [`ItemKey`]
/// 2. Pads the plaintext to the nearest size bucket
/// 3. Encrypts the padded data with AES-256-GCM (random nonce, AAD = `"ldgr-item-seal-v1"`)
/// 4. Wraps the item key with the vault key
///
/// # Errors
///
/// Returns `CryptoError::EncryptionFailed` if AES-GCM encryption fails,
/// or `CryptoError::WrapFailed` if key wrapping fails.
pub fn encrypt_item(vault_key: &VaultKey, plaintext: &[u8]) -> Result<SealedEnvelope, CryptoError> {
    let item_key = ItemKey::generate();
    let padded = pad_to_bucket(plaintext);

    let cipher = Aes256Gcm::new_from_slice(item_key.as_bytes())
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let payload = aes_gcm::aead::Payload {
        msg: &padded,
        aad: ITEM_SEAL_AAD,
    };

    let ciphertext = cipher
        .encrypt(nonce, payload)
        .map_err(|_| CryptoError::EncryptionFailed("AES-GCM encryption failed".into()))?;

    let wrapped_ik = wrap_item_key(vault_key, &item_key)?;

    Ok(SealedEnvelope {
        version: ENVELOPE_VERSION,
        wrapped_ik,
        nonce: nonce_bytes,
        ciphertext,
    })
}

/// Decrypt a sealed envelope, returning the original plaintext.
///
/// 1. Validates the envelope version
/// 2. Unwraps the item key using the vault key
/// 3. Decrypts the ciphertext with AES-256-GCM
/// 4. Removes size-bucket padding
///
/// # Errors
///
/// Returns `CryptoError::InvalidParams` if the version is unsupported,
/// `CryptoError::UnwrapFailed` if key unwrapping fails,
/// or `CryptoError::DecryptionFailed` if decryption or unpadding fails.
pub fn decrypt_item(
    vault_key: &VaultKey,
    envelope: &SealedEnvelope,
) -> Result<Vec<u8>, CryptoError> {
    if envelope.version != ENVELOPE_VERSION {
        return Err(CryptoError::InvalidParams(format!(
            "unsupported envelope version: {}",
            envelope.version
        )));
    }

    let item_key = unwrap_item_key(vault_key, &envelope.wrapped_ik)?;

    let cipher = Aes256Gcm::new_from_slice(item_key.as_bytes())
        .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))?;
    let nonce = Nonce::from_slice(&envelope.nonce);

    let payload = aes_gcm::aead::Payload {
        msg: &envelope.ciphertext,
        aad: ITEM_SEAL_AAD,
    };

    let padded = cipher
        .decrypt(nonce, payload)
        .map_err(|_| CryptoError::DecryptionFailed("AES-GCM decryption failed".into()))?;

    let plaintext = unpad(&padded)?;
    Ok(plaintext.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Padding tests ---

    #[test]
    fn padded_size_buckets() {
        // Empty payload → 4 bytes prefix → 512
        assert_eq!(padded_size(0), 512);
        // 1 byte payload → 5 bytes total → 512
        assert_eq!(padded_size(1), 512);
        // 508 bytes payload → 512 total (exact fit)
        assert_eq!(padded_size(508), 512);
        // 509 bytes → 513 total → 2048
        assert_eq!(padded_size(509), 2_048);
        // 2044 bytes → 2048 total (exact fit)
        assert_eq!(padded_size(2_044), 2_048);
        // 2045 bytes → 2049 total → 8192
        assert_eq!(padded_size(2_045), 8_192);
        // 8188 bytes → 8192 total (exact fit)
        assert_eq!(padded_size(8_188), 8_192);
        // 8189 bytes → 8193 total → 32768
        assert_eq!(padded_size(8_189), 32_768);
        // 32764 bytes → 32768 total (exact fit)
        assert_eq!(padded_size(32_764), 32_768);
        // 32765 bytes → 32769 total → 65536 (next 32KB multiple)
        assert_eq!(padded_size(32_765), 65_536);
    }

    #[test]
    fn pad_unpad_round_trip() {
        let data = b"hello, world!";
        let padded = pad_to_bucket(data);
        assert_eq!(padded.len(), 512);
        let unpadded = unpad(&padded).unwrap();
        assert_eq!(unpadded, data);
    }

    #[test]
    fn pad_empty_payload() {
        let padded = pad_to_bucket(b"");
        assert_eq!(padded.len(), 512);
        let unpadded = unpad(&padded).unwrap();
        assert!(unpadded.is_empty());
    }

    #[test]
    fn pad_exact_bucket_boundary() {
        // 508 bytes of payload + 4 bytes prefix = 512 exactly
        let data = vec![0xAB; 508];
        let padded = pad_to_bucket(&data);
        assert_eq!(padded.len(), 512);
        let unpadded = unpad(&padded).unwrap();
        assert_eq!(unpadded, &data[..]);
    }

    #[test]
    fn pad_one_over_bucket_boundary() {
        // 509 bytes of payload + 4 bytes prefix = 513 → next bucket 2048
        let data = vec![0xCD; 509];
        let padded = pad_to_bucket(&data);
        assert_eq!(padded.len(), 2_048);
        let unpadded = unpad(&padded).unwrap();
        assert_eq!(unpadded, &data[..]);
    }

    #[test]
    fn unpad_rejects_truncated_data() {
        let result = unpad(&[0, 0, 0]);
        assert!(result.is_err());
    }

    #[test]
    fn unpad_rejects_invalid_length() {
        // Length prefix says 100 but only 10 bytes of data follow
        let mut bad = vec![0, 0, 0, 100];
        bad.extend_from_slice(&[0; 10]);
        let result = unpad(&bad);
        assert!(result.is_err());
    }

    // --- Encryption round-trip tests ---

    #[test]
    fn encrypt_decrypt_round_trip() {
        let vk = VaultKey::generate();
        let plaintext = b"Test transaction: 2024-01-15 Groceries $42.50";

        let envelope = encrypt_item(&vk, plaintext).unwrap();
        let decrypted = decrypt_item(&vk, &envelope).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_decrypt_empty_payload() {
        let vk = VaultKey::generate();
        let envelope = encrypt_item(&vk, b"").unwrap();
        let decrypted = decrypt_item(&vk, &envelope).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn encrypt_decrypt_large_payload() {
        let vk = VaultKey::generate();
        let plaintext = vec![0x42; 50_000]; // > 32KB, triggers 64KB bucket

        let envelope = encrypt_item(&vk, &plaintext).unwrap();
        let decrypted = decrypt_item(&vk, &envelope).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn each_encryption_produces_different_output() {
        let vk = VaultKey::generate();
        let plaintext = b"same data encrypted twice";

        let env1 = encrypt_item(&vk, plaintext).unwrap();
        let env2 = encrypt_item(&vk, plaintext).unwrap();

        // Different item keys → different wrapped IKs
        assert_ne!(env1.wrapped_ik.ciphertext, env2.wrapped_ik.ciphertext);
        // Different nonces
        assert_ne!(env1.nonce, env2.nonce);
        // Different ciphertext
        assert_ne!(env1.ciphertext, env2.ciphertext);

        // Both decrypt correctly
        assert_eq!(decrypt_item(&vk, &env1).unwrap(), plaintext);
        assert_eq!(decrypt_item(&vk, &env2).unwrap(), plaintext);
    }

    // --- Tamper detection tests ---

    #[test]
    fn tampered_ciphertext_detected() {
        let vk = VaultKey::generate();
        let mut envelope = encrypt_item(&vk, b"sensitive data").unwrap();

        if let Some(byte) = envelope.ciphertext.first_mut() {
            *byte ^= 0xFF;
        }
        assert!(decrypt_item(&vk, &envelope).is_err());
    }

    #[test]
    fn tampered_nonce_detected() {
        let vk = VaultKey::generate();
        let mut envelope = encrypt_item(&vk, b"sensitive data").unwrap();

        envelope.nonce[0] ^= 0xFF;
        assert!(decrypt_item(&vk, &envelope).is_err());
    }

    #[test]
    fn tampered_wrapped_ik_detected() {
        let vk = VaultKey::generate();
        let mut envelope = encrypt_item(&vk, b"sensitive data").unwrap();

        if let Some(byte) = envelope.wrapped_ik.ciphertext.first_mut() {
            *byte ^= 0xFF;
        }
        assert!(decrypt_item(&vk, &envelope).is_err());
    }

    #[test]
    fn wrong_vault_key_fails_decrypt() {
        let vk1 = VaultKey::generate();
        let vk2 = VaultKey::generate();

        let envelope = encrypt_item(&vk1, b"secret").unwrap();
        assert!(decrypt_item(&vk2, &envelope).is_err());
    }

    #[test]
    fn unsupported_version_rejected() {
        let vk = VaultKey::generate();
        let mut envelope = encrypt_item(&vk, b"data").unwrap();
        envelope.version = 99;
        let result = decrypt_item(&vk, &envelope);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("unsupported envelope version"),);
    }

    // --- Serialization round-trip ---

    #[test]
    fn envelope_serializes_to_json() {
        let vk = VaultKey::generate();
        let plaintext = b"JSON round-trip test";

        let envelope = encrypt_item(&vk, plaintext).unwrap();
        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: SealedEnvelope = serde_json::from_str(&json).unwrap();

        let decrypted = decrypt_item(&vk, &deserialized).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    // --- Property-based tests ---

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn encrypt_decrypt_arbitrary_payload(data in proptest::collection::vec(any::<u8>(), 0..65_536)) {
                let vk = VaultKey::generate();
                let envelope = encrypt_item(&vk, &data).unwrap();
                let decrypted = decrypt_item(&vk, &envelope).unwrap();
                prop_assert_eq!(decrypted, data);
            }

            #[test]
            fn padded_size_is_always_valid_bucket(len in 0_usize..100_000) {
                let size = padded_size(len);
                // Must be >= length prefix + payload
                prop_assert!(size >= LENGTH_PREFIX_LEN + len);
                // Must be one of the fixed buckets or a 32KB multiple
                let valid = BUCKETS.contains(&size) || (size > *BUCKETS.last().unwrap() && size % LARGEST_BUCKET == 0);
                prop_assert!(valid, "Invalid bucket size: {}", size);
            }

            #[test]
            fn pad_unpad_round_trip_arbitrary(data in proptest::collection::vec(any::<u8>(), 0..65_536)) {
                let padded = pad_to_bucket(&data);
                let unpadded = unpad(&padded).unwrap();
                prop_assert_eq!(unpadded, &data[..]);
            }
        }
    }
}
