//! Pure batch-blob framing: [`EventBatch`] ↔ canonical encrypted blob.
//!
//! This is the *only* definition of the on-the-wire sync blob layout:
//!
//! ```text
//! blob = json(SealedEnvelope) = encrypt_item(vault_key, json(EventBatch))
//! ```
//!
//! It is pure computation (crypto + serde only — **no sqlite, no networking**),
//! so it compiles to WASM and is shared by every client. The sqlite-gated
//! [`crate::sync::pipeline`] delegates its private seal/open helpers here, and
//! the WASM host (`ldgr-wasm`) calls the `*_with_session_key` variants directly.
//! Keeping a single implementation guarantees the bytes stay cross-decryptable
//! across CLI / iOS / web.
//!
//! The [`VaultKey`] type is deliberately non-constructible outside `ldgr-core`
//! (`VaultKey::from_bytes` is `pub(crate)`). FFI/WASM hosts only ever hold the
//! raw 32-byte session key (exported via
//! [`crate::crypto::UnlockedVault::export_session_key`]), so the
//! `*_with_session_key` wrappers rebuild the key inside the crate. No additional
//! crypto is performed.

use crate::crypto::{CryptoError, SealedEnvelope, VaultKey, decrypt_item, encrypt_item};

use super::events::{EventBatch, deserialize_batch, serialize_batch};

/// Errors from sealing/opening a batch blob.
#[derive(Debug, thiserror::Error)]
pub enum FramingError {
    /// Encryption or decryption failed (wrong key, tampered blob).
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),
    /// The batch or sealed envelope could not be (de)serialized.
    #[error("blob format error: {0}")]
    Format(String),
}

/// Seal a batch into the canonical blob: `json(encrypt_item(vk, json(batch)))`.
///
/// # Errors
///
/// Returns [`FramingError::Format`] if the batch or envelope cannot be
/// serialized, or [`FramingError::Crypto`] if encryption fails.
pub fn seal_batch(vault_key: &VaultKey, batch: &EventBatch) -> Result<Vec<u8>, FramingError> {
    let plaintext = serialize_batch(batch).map_err(FramingError::Format)?;
    let envelope = encrypt_item(vault_key, &plaintext)?;
    serde_json::to_vec(&envelope)
        .map_err(|e| FramingError::Format(format!("failed to serialize sealed envelope: {e}")))
}

/// Inverse of [`seal_batch`]: decrypt and deserialize a canonical blob.
///
/// # Errors
///
/// Returns [`FramingError::Format`] if the envelope or batch cannot be parsed,
/// or [`FramingError::Crypto`] if decryption fails.
pub fn open_batch(vault_key: &VaultKey, ciphertext: &[u8]) -> Result<EventBatch, FramingError> {
    let envelope: SealedEnvelope = serde_json::from_slice(ciphertext)
        .map_err(|e| FramingError::Format(format!("failed to parse sealed envelope: {e}")))?;
    let plaintext = decrypt_item(vault_key, &envelope)?;
    deserialize_batch(&plaintext).map_err(FramingError::Format)
}

/// Like [`seal_batch`], but accepting the raw 32-byte vault session key (as
/// exported by [`crate::crypto::UnlockedVault::export_session_key`]) instead of
/// a [`VaultKey`]. Intended for FFI/WASM hosts that cannot construct a
/// [`VaultKey`] directly.
///
/// # Errors
///
/// Propagates any [`FramingError`] from [`seal_batch`].
pub fn seal_batch_with_session_key(
    session_key: &[u8; 32],
    batch: &EventBatch,
) -> Result<Vec<u8>, FramingError> {
    seal_batch(&VaultKey::from_bytes(*session_key), batch)
}

/// Like [`open_batch`], but accepting the raw 32-byte vault session key instead
/// of a [`VaultKey`]. Intended for FFI/WASM hosts.
///
/// # Errors
///
/// Propagates any [`FramingError`] from [`open_batch`].
pub fn open_batch_with_session_key(
    session_key: &[u8; 32],
    ciphertext: &[u8],
) -> Result<EventBatch, FramingError> {
    open_batch(&VaultKey::from_bytes(*session_key), ciphertext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::events::{EntityType, Operation, SyncEvent, VectorClock, create_batch};

    fn sample_batch() -> EventBatch {
        let event = SyncEvent {
            id: "evt1".into(),
            device_id: "dev1".into(),
            lamport_clock: 1,
            entity_type: EntityType::Transaction,
            entity_id: "txn1".into(),
            operation: Operation::Create,
            payload: b"{\"id\":\"txn1\"}".to_vec(),
            version: 1,
            created_at: "2024-01-15T00:00:00Z".into(),
        };
        create_batch("dev1", vec![event], &VectorClock::default())
    }

    #[test]
    fn session_key_round_trip() {
        let key = [7u8; 32];
        let batch = sample_batch();
        let blob = seal_batch_with_session_key(&key, &batch).unwrap();
        let restored = open_batch_with_session_key(&key, &blob).unwrap();
        assert_eq!(restored.events.len(), 1);
        assert_eq!(restored.events[0].entity_id, "txn1");
        assert_eq!(restored.events[0].payload, b"{\"id\":\"txn1\"}");
    }

    #[test]
    fn vault_key_and_session_key_produce_same_format() {
        // A blob sealed via VaultKey must open via the session-key path and
        // vice versa — the framing is identical.
        let key = [0x42u8; 32];
        let vk = VaultKey::from_bytes(key);
        let batch = sample_batch();

        let via_vk = seal_batch(&vk, &batch).unwrap();
        let opened = open_batch_with_session_key(&key, &via_vk).unwrap();
        assert_eq!(opened.events[0].entity_id, "txn1");

        let via_sk = seal_batch_with_session_key(&key, &batch).unwrap();
        let opened2 = open_batch(&vk, &via_sk).unwrap();
        assert_eq!(opened2.events[0].entity_id, "txn1");
    }

    #[test]
    fn wrong_key_fails_to_open() {
        let batch = sample_batch();
        let blob = seal_batch_with_session_key(&[1u8; 32], &batch).unwrap();
        assert!(open_batch_with_session_key(&[2u8; 32], &blob).is_err());
    }

    #[test]
    fn malformed_blob_is_format_error() {
        let err = open_batch_with_session_key(&[0u8; 32], b"not json").unwrap_err();
        assert!(matches!(err, FramingError::Format(_)));
    }
}
