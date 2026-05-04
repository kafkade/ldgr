use std::fmt;

use rand::RngCore;
use zeroize::{Zeroize, ZeroizeOnDrop};

const KEY_LEN: usize = 32; // 256-bit

macro_rules! define_key {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Zeroize, ZeroizeOnDrop)]
        pub struct $name([u8; KEY_LEN]);

        #[allow(dead_code)]
        impl $name {
            /// Create from raw bytes. Caller is responsible for ensuring correct derivation.
            pub(crate) fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
                Self(bytes)
            }

            /// Access the raw key material.
            pub(crate) fn as_bytes(&self) -> &[u8; KEY_LEN] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name))
                    .field(&"[REDACTED]")
                    .finish()
            }
        }
    };
}

define_key!(
    /// 256-bit master key derived from the user's password via Argon2id.
    /// This key is never stored — it exists only in memory during a session.
    MasterKey
);

define_key!(
    /// Authentication key derived from the master key via HKDF.
    /// Used for SRP-6a server authentication.
    AuthKey
);

define_key!(
    /// Master encryption key derived from the master key via HKDF.
    /// Used to wrap/unwrap the vault key.
    MasterEncryptionKey
);

define_key!(
    /// Vault key — random 256-bit key that encrypts item keys.
    /// Wrapped by MEK for normal access and by recovery key for emergency access.
    VaultKey
);

define_key!(
    /// Per-item encryption key — random 256-bit key for encrypting a single vault item.
    /// Wrapped by the vault key.
    ItemKey
);

define_key!(
    /// Recovery key — random 256-bit key displayed to user as an emergency kit.
    /// Can unwrap the vault key when the master password is lost.
    RecoveryKey
);

impl VaultKey {
    /// Generate a new random vault key.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }
}

impl ItemKey {
    /// Generate a new random item key.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }
}

impl RecoveryKey {
    /// Generate a new random recovery key.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_master_key() {
        let key = MasterKey::from_bytes([0xAB; KEY_LEN]);
        let debug = format!("{key:?}");
        assert!(
            debug.contains("[REDACTED]"),
            "Debug must not leak key material"
        );
        assert!(!debug.contains("171"), "Debug must not contain byte values");
        assert!(!debug.contains("ab"), "Debug must not contain hex values");
    }

    #[test]
    fn debug_redacts_all_key_types() {
        let mk = MasterKey::from_bytes([1; KEY_LEN]);
        let ak = AuthKey::from_bytes([2; KEY_LEN]);
        let mek = MasterEncryptionKey::from_bytes([3; KEY_LEN]);
        let vk = VaultKey::from_bytes([4; KEY_LEN]);
        let ik = ItemKey::from_bytes([5; KEY_LEN]);
        let rk = RecoveryKey::from_bytes([6; KEY_LEN]);

        for (name, debug) in [
            ("MasterKey", format!("{mk:?}")),
            ("AuthKey", format!("{ak:?}")),
            ("MasterEncryptionKey", format!("{mek:?}")),
            ("VaultKey", format!("{vk:?}")),
            ("ItemKey", format!("{ik:?}")),
            ("RecoveryKey", format!("{rk:?}")),
        ] {
            assert!(
                debug.contains("[REDACTED]"),
                "{name} Debug must contain [REDACTED], got: {debug}"
            );
        }
    }

    #[test]
    fn generated_keys_are_unique() {
        let vk1 = VaultKey::generate();
        let vk2 = VaultKey::generate();
        assert_ne!(vk1.as_bytes(), vk2.as_bytes());

        let ik1 = ItemKey::generate();
        let ik2 = ItemKey::generate();
        assert_ne!(ik1.as_bytes(), ik2.as_bytes());
    }

    #[test]
    fn clone_produces_equal_key() {
        let vk = VaultKey::generate();
        let vk2 = vk.clone();
        assert_eq!(vk.as_bytes(), vk2.as_bytes());
    }
}
