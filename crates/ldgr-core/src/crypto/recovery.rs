//! Recovery key human-readable encoding using Crockford Base32.
//!
//! The recovery key is a 256-bit random key encoded as a 52-character
//! Crockford Base32 string, displayed with dashes every 4 characters
//! for readability. Crockford Base32 excludes I, L, O, U to reduce
//! transcription errors, and normalizes common substitutions on decode.
//!
//! Format: `XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX`

use super::errors::CryptoError;
use super::keys::RecoveryKey;

/// Crockford's Base32 encoding alphabet (excludes I, L, O, U).
const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

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
    let raw = encode_crockford(key.as_bytes());
    raw.as_bytes()
        .chunks(GROUP_SIZE)
        .map(|chunk| {
            // SAFETY: encode_crockford only produces ASCII
            std::str::from_utf8(chunk).expect("Crockford output is always valid UTF-8")
        })
        .collect::<Vec<_>>()
        .join("-")
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

    let bytes = decode_crockford(&clean)?;
    Ok(RecoveryKey::from_bytes(bytes))
}

/// Encode 32 bytes as 52-character Crockford Base32 string.
fn encode_crockford(data: &[u8; 32]) -> String {
    let mut result = String::with_capacity(ENCODED_LEN);
    let mut buffer: u64 = 0;
    let mut bits = 0u32;

    for &byte in data {
        buffer = (buffer << 8) | u64::from(byte);
        bits += 8;

        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1F) as usize;
            result.push(CROCKFORD_ALPHABET[index] as char);
        }
    }

    // 256 bits mod 5 = 1 remaining bit → pad with 4 zero bits
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1F) as usize;
        result.push(CROCKFORD_ALPHABET[index] as char);
    }

    debug_assert_eq!(result.len(), ENCODED_LEN);
    result
}

/// Decode a 52-character Crockford Base32 string to 32 bytes.
fn decode_crockford(s: &str) -> Result<[u8; 32], CryptoError> {
    if s.len() != ENCODED_LEN {
        return Err(CryptoError::InvalidParams(format!(
            "recovery key must be {ENCODED_LEN} characters, got {}",
            s.len()
        )));
    }

    let mut buffer: u64 = 0;
    let mut bits = 0u32;
    let mut result = Vec::with_capacity(32);

    for ch in s.chars() {
        let value = crockford_decode_char(ch)?;
        buffer = (buffer << 5) | u64::from(value);
        bits += 5;

        while bits >= 8 {
            bits -= 8;
            #[allow(clippy::cast_possible_truncation)] // intentional: extracting low byte
            result.push((buffer >> bits) as u8);
        }
    }

    // 52 × 5 = 260 bits = 32 bytes + 4 padding bits (must be zero)
    if bits > 0 {
        let mask = (1u64 << bits) - 1;
        if buffer & mask != 0 {
            return Err(CryptoError::InvalidParams(
                "invalid padding in recovery key".into(),
            ));
        }
    }

    let bytes: [u8; 32] = result
        .try_into()
        .map_err(|_| CryptoError::InvalidParams("decoded recovery key has wrong length".into()))?;

    Ok(bytes)
}

/// Decode a single Crockford Base32 character to its 5-bit value.
///
/// Applies normalization: O → 0, I/L → 1. Rejects U (not in alphabet).
fn crockford_decode_char(c: char) -> Result<u8, CryptoError> {
    match c {
        '0' | 'O' | 'o' => Ok(0),
        '1' | 'I' | 'i' | 'L' | 'l' => Ok(1),
        '2' => Ok(2),
        '3' => Ok(3),
        '4' => Ok(4),
        '5' => Ok(5),
        '6' => Ok(6),
        '7' => Ok(7),
        '8' => Ok(8),
        '9' => Ok(9),
        'A' | 'a' => Ok(10),
        'B' | 'b' => Ok(11),
        'C' | 'c' => Ok(12),
        'D' | 'd' => Ok(13),
        'E' | 'e' => Ok(14),
        'F' | 'f' => Ok(15),
        'G' | 'g' => Ok(16),
        'H' | 'h' => Ok(17),
        'J' | 'j' => Ok(18),
        'K' | 'k' => Ok(19),
        'M' | 'm' => Ok(20),
        'N' | 'n' => Ok(21),
        'P' | 'p' => Ok(22),
        'Q' | 'q' => Ok(23),
        'R' | 'r' => Ok(24),
        'S' | 's' => Ok(25),
        'T' | 't' => Ok(26),
        'V' | 'v' => Ok(27),
        'W' | 'w' => Ok(28),
        'X' | 'x' => Ok(29),
        'Y' | 'y' => Ok(30),
        'Z' | 'z' => Ok(31),
        _ => Err(CryptoError::InvalidParams(format!(
            "invalid character in recovery key: '{c}'"
        ))),
    }
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
                CROCKFORD_ALPHABET.contains(&(ch as u8)),
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
