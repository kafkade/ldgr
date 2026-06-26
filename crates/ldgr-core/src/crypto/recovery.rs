//! Recovery key human-readable encoding using Crockford Base32.
//!
//! The recovery key is a 256-bit random key encoded as a 52-character
//! Crockford Base32 string, displayed with dashes every 4 characters
//! for readability. Crockford Base32 excludes I, L, O, U to reduce
//! transcription errors, and normalizes common substitutions on decode.
//!
//! Format: `XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX`

use super::crockford;
use super::errors::CryptoError;
use super::keys::RecoveryKey;

/// Number of raw key bytes (256 bits).
const KEY_BYTES: usize = 32;

/// Expected length of the base32-encoded recovery key (256 bits → 52 chars).
const ENCODED_LEN: usize = 52;

/// Characters per display group in the formatted output.
const GROUP_SIZE: usize = 4;

/// Encode a recovery key as a human-readable Crockford Base32 string
/// with dashes every 4 characters.
///
/// # Example output
///
/// `A1B2-C3D4-E5F6-G7H8-J9KA-BCDE-FGHJ-KMNP-QRST-VWXY-Z012-3456-789A`
pub fn encode_recovery_key(key: &RecoveryKey) -> String {
    crockford::group(&crockford::encode(key.as_bytes()), GROUP_SIZE)
}

/// Decode a human-readable recovery key string back to a [`RecoveryKey`].
///
/// Accepts any case. Ignores dashes, spaces, and tabs. Applies Crockford
/// normalization: `O`/`o` → `0`, `I`/`i`/`L`/`l` → `1`.
///
/// # Errors
///
/// Returns [`CryptoError::InvalidParams`] if the string contains invalid
/// characters or decodes to the wrong length.
pub fn decode_recovery_key(encoded: &str) -> Result<RecoveryKey, CryptoError> {
    let clean: String = encoded
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();

    if clean.chars().count() != ENCODED_LEN {
        return Err(CryptoError::InvalidParams(format!(
            "recovery key must be {ENCODED_LEN} characters, got {}",
            clean.chars().count()
        )));
    }

    let bytes = crockford::decode(&clean, KEY_BYTES)?;
    let bytes: [u8; KEY_BYTES] = bytes
        .try_into()
        .map_err(|_| CryptoError::InvalidParams("decoded recovery key has wrong length".into()))?;
    Ok(RecoveryKey::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_generated_key() {
        let key = RecoveryKey::generate();
        let encoded = encode_recovery_key(&key);
        let decoded = decode_recovery_key(&encoded).unwrap();
        assert_eq!(key.as_bytes(), decoded.as_bytes());
    }

    #[test]
    fn round_trip_known_bytes() {
        let bytes = [0xAB; 32];
        let key = RecoveryKey::from_bytes(bytes);
        let encoded = encode_recovery_key(&key);
        let decoded = decode_recovery_key(&encoded).unwrap();
        assert_eq!(&bytes, decoded.as_bytes());
    }

    #[test]
    fn round_trip_all_zeros() {
        let key = RecoveryKey::from_bytes([0u8; 32]);
        let encoded = encode_recovery_key(&key);
        let decoded = decode_recovery_key(&encoded).unwrap();
        assert_eq!(key.as_bytes(), decoded.as_bytes());
    }

    #[test]
    fn round_trip_all_ones() {
        let key = RecoveryKey::from_bytes([0xFF; 32]);
        let encoded = encode_recovery_key(&key);
        let decoded = decode_recovery_key(&encoded).unwrap();
        assert_eq!(key.as_bytes(), decoded.as_bytes());
    }

    #[test]
    fn encoded_format_has_dashes() {
        let key = RecoveryKey::generate();
        let encoded = encode_recovery_key(&key);
        // 52 chars + 12 dashes (13 groups of 4, 12 separators)
        assert_eq!(encoded.len(), ENCODED_LEN + 12);
        assert!(encoded.contains('-'));
    }

    #[test]
    fn encoded_uses_only_crockford_chars() {
        let key = RecoveryKey::generate();
        let encoded = encode_recovery_key(&key);
        let clean: String = encoded.chars().filter(|c| *c != '-').collect();
        for ch in clean.chars() {
            assert!(
                crockford::ALPHABET.contains(&(ch as u8)),
                "unexpected character: '{ch}'"
            );
        }
    }

    #[test]
    fn decode_is_case_insensitive() {
        let key = RecoveryKey::generate();
        let encoded = encode_recovery_key(&key);
        let lower = decode_recovery_key(&encoded.to_lowercase()).unwrap();
        let upper = decode_recovery_key(&encoded.to_uppercase()).unwrap();
        assert_eq!(key.as_bytes(), lower.as_bytes());
        assert_eq!(key.as_bytes(), upper.as_bytes());
    }

    #[test]
    fn decode_ignores_whitespace() {
        let key = RecoveryKey::generate();
        let encoded = encode_recovery_key(&key);
        let with_spaces = encoded.replace('-', " ");
        let decoded = decode_recovery_key(&with_spaces).unwrap();
        assert_eq!(key.as_bytes(), decoded.as_bytes());
    }

    #[test]
    fn decode_normalizes_confusable_chars() {
        let key = RecoveryKey::from_bytes([0u8; 32]);
        let encoded = encode_recovery_key(&key);
        // All zeros encodes to all '0' characters
        // Replace some 0s with O, I, L (should decode back to 0 and 1 respectively)
        let original_clean: String = encoded.chars().filter(|c| *c != '-').collect();
        assert!(original_clean.chars().all(|c| c == '0'));

        // O → 0 normalization
        let with_o = encoded.replace('0', "O");
        let decoded = decode_recovery_key(&with_o).unwrap();
        assert_eq!(key.as_bytes(), decoded.as_bytes());
    }

    #[test]
    fn decode_rejects_invalid_chars() {
        let result =
            decode_recovery_key("!@#$-EFGH-IJKL-MNOP-QRST-UVWX-YZAB-CDEF-GHIJ-KLMN-OPQR-STUV-WXYZ");
        assert!(result.is_err());
    }

    #[test]
    fn decode_rejects_wrong_length() {
        let result = decode_recovery_key("ABCD-EFGH");
        assert!(result.is_err());
    }

    #[test]
    fn generated_keys_produce_different_encodings() {
        let k1 = RecoveryKey::generate();
        let k2 = RecoveryKey::generate();
        assert_ne!(encode_recovery_key(&k1), encode_recovery_key(&k2));
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn round_trip_arbitrary_bytes(bytes in proptest::array::uniform32(any::<u8>())) {
                let key = RecoveryKey::from_bytes(bytes);
                let encoded = encode_recovery_key(&key);
                let decoded = decode_recovery_key(&encoded).unwrap();
                prop_assert_eq!(key.as_bytes(), decoded.as_bytes());
            }
        }
    }
}
