//! Cryptographic primitives for ldgr.
//!
//! Key hierarchy: Password → Argon2id → MK → HKDF → MEK → wraps VK → wraps IKs
//!
//! All key types implement [`Zeroize`] and [`ZeroizeOnDrop`].
//! [`Debug`] implementations redact secret values.

mod envelope;
mod errors;
mod kdf;
mod keys;
mod wrap;

pub use envelope::{SealedEnvelope, decrypt_item, encrypt_item};
pub use errors::CryptoError;
pub use kdf::{Argon2Params, derive_auth_key, derive_encryption_key, derive_master_key};
pub use keys::{AuthKey, ItemKey, MasterEncryptionKey, MasterKey, RecoveryKey, VaultKey};
pub use wrap::{
    WrappedKey, unwrap_item_key, unwrap_vault_key, unwrap_vault_key_with_recovery, wrap_item_key,
    wrap_vault_key, wrap_vault_key_with_recovery,
};
