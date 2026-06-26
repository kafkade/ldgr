//! Cryptographic primitives for ldgr.
//!
//! Key hierarchy: Password → Argon2id → MK → HKDF → MEK → wraps VK → wraps IKs
//!
//! All key types implement [`Zeroize`] and [`ZeroizeOnDrop`].
//! [`Debug`] implementations redact secret values.

mod crockford;
mod envelope;
mod errors;
mod kdf;
mod keys;
mod recovery;
mod secret_key;
#[cfg(feature = "sync")]
mod two_skd;
mod vault;
mod wrap;

pub use envelope::{SealedEnvelope, decrypt_item, encrypt_item};
pub use errors::CryptoError;
pub use kdf::{Argon2Params, derive_auth_key, derive_encryption_key, derive_master_key};
pub use keys::{AuthKey, ItemKey, MasterEncryptionKey, MasterKey, RecoveryKey, VaultKey};
pub use recovery::{decode_recovery_key, encode_recovery_key};
pub use secret_key::SecretKey;
#[cfg(feature = "sync")]
pub(crate) use two_skd::derive_x_seed;
pub use vault::{
    UnlockedVault, VaultHeader, VaultMetadata, create_vault, open_vault, recover_vault,
    restore_vault_from_session, serialize_vault, validate_vault, verify_recovery_key,
};
pub use wrap::{
    WrappedKey, unwrap_item_key, unwrap_vault_key, unwrap_vault_key_with_recovery, wrap_item_key,
    wrap_vault_key, wrap_vault_key_with_recovery,
};

#[cfg(feature = "test-vectors")]
#[doc(hidden)]
pub use envelope::encrypt_item_with;
#[cfg(feature = "test-vectors")]
#[doc(hidden)]
pub use vault::{serialize_parts, serialize_sealed_envelope, serialize_wrapped_key};
#[cfg(feature = "test-vectors")]
#[doc(hidden)]
pub use wrap::{
    wrap_item_key_with_nonce, wrap_vault_key_with_nonce, wrap_vault_key_with_recovery_with_nonce,
};
