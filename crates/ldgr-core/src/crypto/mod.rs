//! Cryptographic primitives for ldgr.
//!
//! Key hierarchy: Password → Argon2id → MK → HKDF → MEK → wraps VK → wraps IKs
//!
//! All key types implement [`Zeroize`] and [`ZeroizeOnDrop`].
//! [`Debug`] implementations redact secret values.

// Modules will be added as crypto features are implemented:
// pub mod keys;          // Key types (MK, MEK, VK, IK)
// pub mod key_hierarchy; // Derivation and wrapping
// pub mod vault;         // Vault encryption/decryption
// pub mod padding;       // Size-bucket padding
