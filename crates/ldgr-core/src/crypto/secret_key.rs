//! Account **Secret Key** — the second factor in two-secret key derivation
//! (2SKD, see ADR-008).
//!
//! The Secret Key is a high-entropy, client-generated account key. Combined
//! with the master password it derives the SRP-6a verifier the server stores
//! (`crypto/two_skd.rs`). The server never receives it, which raises the
//! offline brute-force floor on a stolen verifier to ≥128 bits regardless of
//! password strength.
//!
//! **Local-first guarantee (ADR-008 Decision 3):** the Secret Key strengthens
//! *server auth / sync only*. It never participates in vault encryption or
//! decryption — a vault opens offline with the password (or the vault recovery
//! key) alone.
//!
//! # Text format
//!
//! ```text
//! A1-7QK2R9-XJ4F-NK8H-2W6P-...   (dashes/spaces ignored on decode)
//! └┬┘ └──┬─┘ └────────┬──────┘
//!  │     │            └ ≥128-bit random body (26 Crockford chars)
//!  │     └ account-id hint (6 Crockford chars = first 30 bits of account_id)
//!  └ version prefix: 'A' (ldgr account key) + '1' (scheme v1)
//! ```
//!
//! This is intentionally distinct from the bare 52-char vault recovery key so
//! the two artifacts are never confused.

use std::fmt;

use rand::Rng;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::crockford;
use super::errors::CryptoError;

/// Version prefix: `A` (ldgr account key) + `1` (scheme version 1).
const VERSION_PREFIX: &str = "A1";

/// Account-id hint length in Crockford characters (first 30 bits of `account_id`).
const HINT_CHARS: usize = 6;

/// Random body length in bytes (128 bits ≥ the 128-bit minimum).
const BODY_BYTES: usize = 16;

/// Random body length in Crockford characters (`ceil(16 * 8 / 5)` = 26).
const BODY_CHARS: usize = 26;

/// Total significant characters once dashes/whitespace are stripped.
const TOTAL_CHARS: usize = VERSION_PREFIX.len() + HINT_CHARS + BODY_CHARS;

/// Display grouping for the body portion.
const GROUP_SIZE: usize = 4;

/// An account Secret Key: a versioned, human-transcribable account key that
/// forms the second factor of 2SKD.
///
/// The random `body` is secret material and is zeroized on drop; the
/// `account_hint` is a non-secret pairing aid.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey {
    /// Non-secret 6-char account-id hint (canonical uppercase Crockford).
    #[zeroize(skip)]
    account_hint: String,
    /// ≥128-bit random body — the entropy that protects the verifier.
    body: [u8; BODY_BYTES],
}

impl SecretKey {
    /// Generate a new random Secret Key bound (by its hint) to `account_id`.
    ///
    /// Uses the same CSPRNG as [`super::keys::RecoveryKey::generate`].
    #[must_use]
    pub fn generate(account_id: Uuid) -> Self {
        let mut body = [0u8; BODY_BYTES];
        rand::rng().fill_bytes(&mut body);
        Self {
            account_hint: hint_from_uuid(&account_id),
            body,
        }
    }

    /// The non-secret 6-character account-id hint (canonical uppercase).
    #[must_use]
    pub fn account_hint(&self) -> &str {
        &self.account_hint
    }

    /// The decoded ≥128-bit random body (`SK_body`). Secret material.
    #[cfg(any(test, feature = "sync"))]
    pub(crate) fn body(&self) -> &[u8; BODY_BYTES] {
        &self.body
    }

    /// Encode the Secret Key as its canonical dash-grouped text form.
    #[must_use]
    pub fn encode(&self) -> String {
        let body = crockford::group(&crockford::encode(&self.body), GROUP_SIZE);
        format!("{VERSION_PREFIX}-{}-{body}", self.account_hint)
    }

    /// Parse a Secret Key from its text form.
    ///
    /// Case-insensitive; whitespace and dashes are ignored; Crockford
    /// confusables (`O→0`, `I`/`L→1`) are normalized.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidParams`] if the version prefix, length, or
    /// character set is invalid.
    pub fn parse(s: &str) -> Result<Self, CryptoError> {
        let clean: Vec<char> = s
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect();

        if clean.len() != TOTAL_CHARS {
            return Err(CryptoError::InvalidParams(format!(
                "secret key must be {TOTAL_CHARS} characters (excluding dashes/spaces), got {}",
                clean.len()
            )));
        }

        // Version prefix "A1" (normalization maps e.g. 'I'/'L' → 1).
        if crockford::decode_char(clean[0])? != 10 || crockford::decode_char(clean[1])? != 1 {
            return Err(CryptoError::InvalidParams(
                "invalid secret key version prefix (expected 'A1')".into(),
            ));
        }

        let hint: String = clean[2..2 + HINT_CHARS].iter().collect();
        let account_hint = crockford::normalize(&hint)?;

        let body_str: String = clean[2 + HINT_CHARS..].iter().collect();
        let bytes = crockford::decode(&body_str, BODY_BYTES)?;
        let body: [u8; BODY_BYTES] = bytes.try_into().map_err(|_| {
            CryptoError::InvalidParams("decoded secret key body wrong length".into())
        })?;

        Ok(Self { account_hint, body })
    }
}

impl fmt::Display for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretKey")
            .field("account_hint", &self.account_hint)
            .field("body", &"[REDACTED]")
            .finish()
    }
}

/// Derive the 6-character account-id hint from the first 30 bits of a UUID.
fn hint_from_uuid(id: &Uuid) -> String {
    let mut s = String::with_capacity(HINT_CHARS);
    let mut buffer: u64 = 0;
    let mut bits = 0u32;

    for &byte in id.as_bytes() {
        buffer = (buffer << 8) | u64::from(byte);
        bits += 8;
        while bits >= 5 && s.len() < HINT_CHARS {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1F) as usize;
            s.push(crockford::ALPHABET[index] as char);
        }
        if s.len() >= HINT_CHARS {
            break;
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_uuid() -> Uuid {
        Uuid::from_bytes([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB,
            0xCD, 0xEF,
        ])
    }

    #[test]
    fn generate_has_version_and_hint() {
        let sk = SecretKey::generate(fixed_uuid());
        let text = sk.encode();
        assert!(text.starts_with("A1-"));
        assert_eq!(sk.account_hint().len(), HINT_CHARS);
        assert!(text.contains(sk.account_hint()));
    }

    #[test]
    fn hint_is_deterministic_for_account_id() {
        let a = SecretKey::generate(fixed_uuid());
        let b = SecretKey::generate(fixed_uuid());
        // Same account → same hint, but different random bodies.
        assert_eq!(a.account_hint(), b.account_hint());
        assert_ne!(a.body(), b.body());
    }

    #[test]
    fn round_trip_encode_parse() {
        let sk = SecretKey::generate(fixed_uuid());
        let parsed = SecretKey::parse(&sk.encode()).unwrap();
        assert_eq!(parsed.account_hint(), sk.account_hint());
        assert_eq!(parsed.body(), sk.body());
    }

    #[test]
    fn parse_is_case_insensitive() {
        let sk = SecretKey::generate(fixed_uuid());
        let text = sk.encode();
        let lower = SecretKey::parse(&text.to_lowercase()).unwrap();
        let upper = SecretKey::parse(&text.to_uppercase()).unwrap();
        assert_eq!(lower.body(), sk.body());
        assert_eq!(upper.body(), sk.body());
    }

    #[test]
    fn parse_ignores_whitespace_and_dashes() {
        let sk = SecretKey::generate(fixed_uuid());
        let spaced = sk.encode().replace('-', "  ");
        let parsed = SecretKey::parse(&spaced).unwrap();
        assert_eq!(parsed.body(), sk.body());
    }

    #[test]
    fn parse_normalizes_confusable_chars() {
        let sk = SecretKey::generate(fixed_uuid());
        let text = sk.encode();
        // Substitute confusables 0→O and 1→L throughout; both normalize back.
        let confused: String = text
            .chars()
            .map(|c| match c {
                '0' => 'O',
                '1' => 'L',
                other => other,
            })
            .collect();
        let parsed = SecretKey::parse(&confused).unwrap();
        assert_eq!(parsed.body(), sk.body());
    }

    #[test]
    fn parse_rejects_wrong_length() {
        assert!(SecretKey::parse("A1-7QK2R9-XJ4F").is_err());
    }

    #[test]
    fn parse_rejects_bad_version() {
        let sk = SecretKey::generate(fixed_uuid());
        // Replace 'A' prefix with 'B' (a valid Crockford char, wrong version).
        let bad = format!("B1{}", &sk.encode()[2..]);
        let err = SecretKey::parse(&bad).unwrap_err();
        assert!(matches!(err, CryptoError::InvalidParams(_)));
    }

    #[test]
    fn parse_rejects_invalid_chars() {
        let sk = SecretKey::generate(fixed_uuid());
        let text = sk.encode();
        // 'U' is excluded from the Crockford alphabet.
        let bad = format!("{}U", &text[..text.len() - 1]);
        assert!(SecretKey::parse(&bad).is_err());
    }

    #[test]
    fn distinct_from_recovery_key_format() {
        // A bare 52-char recovery key (no A1- prefix) must not parse as a
        // Secret Key, and vice versa.
        let rk = super::super::encode_recovery_key(&super::super::keys::RecoveryKey::generate());
        assert!(SecretKey::parse(&rk).is_err());
        // Conversely, a Secret Key text form is 34 chars (excl. dashes), not the
        // recovery key's 52, so they are not confusable.
        let sk = SecretKey::generate(fixed_uuid());
        assert!(super::super::decode_recovery_key(&sk.encode()).is_err());
    }

    #[test]
    fn debug_redacts_body() {
        use std::fmt::Write as _;
        let sk = SecretKey::generate(fixed_uuid());
        let dbg = format!("{sk:?}");
        assert!(dbg.contains("[REDACTED]"));
        // The hint is non-secret and may appear, but raw body bytes must not.
        let mut body_hex = String::new();
        for b in sk.body() {
            write!(body_hex, "{b:02x}").unwrap();
        }
        assert!(!dbg.contains(&body_hex));
    }

    #[test]
    fn generated_keys_are_unique() {
        let a = SecretKey::generate(fixed_uuid());
        let b = SecretKey::generate(fixed_uuid());
        assert_ne!(a.encode(), b.encode());
    }

    /// LOCAL-FIRST GUARD (ADR-008, Decision 3): the account Secret Key must
    /// never be required to open the local vault. A vault opens offline with
    /// the master password alone, and recovers offline with the vault recovery
    /// key alone — neither path involves a Secret Key.
    #[test]
    fn vault_opens_without_any_secret_key() {
        use super::super::Argon2Params;
        use super::super::vault::{create_vault, open_vault, recover_vault, serialize_vault};

        let password = b"local-first-password";
        let (vault, recovery_key) =
            create_vault(password, "Local Vault", &Argon2Params::test()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();

        // (1) Opens with the password alone — no Secret Key anywhere.
        let opened = open_vault(&bytes, password).unwrap();
        assert_eq!(opened.metadata().name, "Local Vault");

        // (2) Recovers with the vault recovery key alone — still no Secret Key.
        let recovered = recover_vault(
            &bytes,
            &recovery_key,
            b"new-password",
            &Argon2Params::test(),
        )
        .unwrap();
        assert_eq!(recovered.metadata().name, "Local Vault");

        // Generating a Secret Key for the same identity changes nothing about
        // the vault: it remains openable with the original password.
        let _sk = SecretKey::generate(fixed_uuid());
        let reopened = open_vault(&bytes, password).unwrap();
        assert_eq!(reopened.metadata().name, "Local Vault");
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn round_trip_arbitrary_account_id(bytes in proptest::array::uniform16(any::<u8>())) {
                let sk = SecretKey::generate(Uuid::from_bytes(bytes));
                let parsed = SecretKey::parse(&sk.encode()).unwrap();
                prop_assert_eq!(parsed.body(), sk.body());
                prop_assert_eq!(parsed.account_hint(), sk.account_hint());
            }
        }
    }
}
