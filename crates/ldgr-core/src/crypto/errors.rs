use thiserror::Error;

/// Errors that can occur during cryptographic operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("key derivation failed: {0}")]
    KeyDerivation(String),

    #[error("key wrapping failed: {0}")]
    WrapFailed(String),

    #[error("key unwrapping failed: authentication or decryption error")]
    UnwrapFailed,

    #[error("encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("invalid parameters: {0}")]
    InvalidParams(String),
}
