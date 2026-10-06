//! Vault identifiers: the stable, unguessable handle a vault is known by on a
//! sync server (ADR-011).
//!
//! Pure computation — generation and validation only. Persistence lives in
//! [`crate::storage::sync::vault_id`] (CLI/native) or in platform storage
//! (iOS `UserDefaults`, web `sync_state`).
//!
//! ## Why not derive the id
//!
//! Earlier clients derived the id as a djb2 hash of the vault *directory path*
//! (`vault_{hash:016x}`). That made the id predictable, enumerable, and — because
//! most users keep the vault at the default path — **identical across accounts**,
//! so the first account to claim it on a shared server locked every other account
//! out of that id permanently. Identifiers are now random, so two vaults collide
//! only with probability 2⁻¹²⁸.
//!
//! Deriving the id from the vault key was considered and rejected: it would
//! couple a server-visible identifier to key material and break under any future
//! vault-key rotation.

use rand::Rng;

/// Prefix marking a random, ADR-011 vault identifier.
const PREFIX: &str = "v1_";

/// Bytes of entropy in a generated identifier.
const ENTROPY_BYTES: usize = 16;

/// Prefix used by pre-ADR-011 path-derived identifiers.
const LEGACY_PREFIX: &str = "vault_";

/// Maximum accepted identifier length, matching the server's validation.
pub const MAX_VAULT_ID_LEN: usize = 128;

/// Generate a fresh, random vault identifier.
///
/// The result is `v1_` followed by 32 lowercase hex characters (128 bits of
/// CSPRNG entropy) — unguessable, and stable once persisted by the caller.
#[must_use]
pub fn generate_vault_id() -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut bytes = [0u8; ENTROPY_BYTES];
    rand::rng().fill_bytes(&mut bytes);

    let mut id = String::with_capacity(PREFIX.len() + ENTROPY_BYTES * 2);
    id.push_str(PREFIX);
    for b in bytes {
        id.push(char::from(HEX[usize::from(b >> 4)]));
        id.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    id
}

/// Whether `id` is a well-formed identifier the server will accept.
///
/// Accepts both random ADR-011 identifiers and the legacy path-derived ones, so
/// existing vaults keep working. The character set is restricted to
/// `[A-Za-z0-9_-]` because the identifier is interpolated into blob paths and
/// URL path segments.
#[must_use]
pub fn is_valid_vault_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_VAULT_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Whether `id` looks like a pre-ADR-011 path-derived identifier.
///
/// Such identifiers are grandfathered — they keep addressing already-synced
/// data — but clients never mint new ones.
#[must_use]
pub fn is_legacy_vault_id(id: &str) -> bool {
    id.strip_prefix(LEGACY_PREFIX)
        .is_some_and(|rest| rest.len() == 16 && rest.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether `id` is a random identifier minted under ADR-011.
#[must_use]
pub fn is_random_vault_id(id: &str) -> bool {
    id.strip_prefix(PREFIX).is_some_and(|rest| {
        rest.len() == ENTROPY_BYTES * 2 && rest.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn generated_ids_have_the_documented_shape() {
        let id = generate_vault_id();
        assert!(id.starts_with(PREFIX), "missing prefix: {id}");
        assert_eq!(id.len(), PREFIX.len() + 32);
        assert!(is_random_vault_id(&id));
        assert!(is_valid_vault_id(&id));
        assert!(!is_legacy_vault_id(&id));
    }

    #[test]
    fn generated_ids_are_unique() {
        let ids: HashSet<String> = (0..1000).map(|_| generate_vault_id()).collect();
        assert_eq!(ids.len(), 1000, "generated ids collided");
    }

    #[test]
    fn generated_ids_are_not_all_zero() {
        // Guards against an RNG that silently yields zeroed buffers.
        let id = generate_vault_id();
        assert_ne!(id, format!("{PREFIX}{}", "0".repeat(32)));
    }

    #[test]
    fn legacy_ids_are_recognized_and_still_valid() {
        let legacy = "vault_0123456789abcdef";
        assert!(is_legacy_vault_id(legacy));
        assert!(is_valid_vault_id(legacy));
        assert!(!is_random_vault_id(legacy));
    }

    #[test]
    fn legacy_detection_rejects_near_misses() {
        assert!(!is_legacy_vault_id("vault_0123456789abcde"), "too short");
        assert!(!is_legacy_vault_id("vault_0123456789abcdef0"), "too long");
        assert!(!is_legacy_vault_id("vault_0123456789abcdeg"), "not hex");
        assert!(!is_legacy_vault_id("v1_0123456789abcdef"));
    }

    #[test]
    fn validation_rejects_empty_oversized_and_path_traversing_ids() {
        assert!(!is_valid_vault_id(""));
        assert!(!is_valid_vault_id(&"a".repeat(MAX_VAULT_ID_LEN + 1)));
        assert!(is_valid_vault_id(&"a".repeat(MAX_VAULT_ID_LEN)));
        assert!(!is_valid_vault_id("../etc/passwd"));
        assert!(!is_valid_vault_id("vault/batches"));
        assert!(!is_valid_vault_id("vault id"));
        assert!(!is_valid_vault_id("vault.id"));
    }
}
