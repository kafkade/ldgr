//! Shared Crockford Base32 helpers.
//!
//! Crockford Base32 excludes `I`, `L`, `O`, `U` to reduce transcription
//! errors and normalizes common confusable substitutions on decode
//! (`O`/`o` → `0`, `I`/`i`/`L`/`l` → `1`). These primitives are reused by
//! both the vault recovery key ([`super::recovery`]) and the account
//! Secret Key ([`super::secret_key`]).

use super::errors::CryptoError;

/// Crockford's Base32 encoding alphabet (excludes I, L, O, U).
pub(crate) const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Number of Crockford characters needed to encode `n` bytes
/// (`ceil(n * 8 / 5)`).
#[must_use]
pub(crate) const fn encoded_len(n: usize) -> usize {
    (n * 8).div_ceil(5)
}

/// Encode bytes as a Crockford Base32 string. Trailing bits (when the input
/// is not a multiple of 5 bits) are zero-padded into the final character.
pub(crate) fn encode(data: &[u8]) -> String {
    let mut result = String::with_capacity(encoded_len(data.len()));
    let mut buffer: u64 = 0;
    let mut bits = 0u32;

    for &byte in data {
        buffer = (buffer << 8) | u64::from(byte);
        bits += 8;

        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1F) as usize;
            result.push(ALPHABET[index] as char);
        }
    }

    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1F) as usize;
        result.push(ALPHABET[index] as char);
    }

    result
}

/// Decode a Crockford Base32 string (already stripped of whitespace and
/// dashes) into exactly `out_len` bytes.
///
/// # Errors
///
/// Returns [`CryptoError::InvalidParams`] if the string has the wrong length,
/// contains an invalid character, or carries non-zero padding bits.
pub(crate) fn decode(s: &str, out_len: usize) -> Result<Vec<u8>, CryptoError> {
    let expected = encoded_len(out_len);
    let actual = s.chars().count();
    if actual != expected {
        return Err(CryptoError::InvalidParams(format!(
            "expected {expected} Crockford characters, got {actual}"
        )));
    }

    let mut buffer: u64 = 0;
    let mut bits = 0u32;
    let mut result = Vec::with_capacity(out_len);

    for ch in s.chars() {
        let value = decode_char(ch)?;
        buffer = (buffer << 5) | u64::from(value);
        bits += 5;

        while bits >= 8 {
            bits -= 8;
            #[allow(clippy::cast_possible_truncation)] // intentional: extracting low byte
            result.push((buffer >> bits) as u8);
        }
    }

    // Any leftover bits are padding and must be zero.
    if bits > 0 {
        let mask = (1u64 << bits) - 1;
        if buffer & mask != 0 {
            return Err(CryptoError::InvalidParams(
                "invalid trailing padding bits".into(),
            ));
        }
    }

    if result.len() != out_len {
        return Err(CryptoError::InvalidParams(
            "decoded to unexpected length".into(),
        ));
    }

    Ok(result)
}

/// Re-encode a Crockford string into canonical (uppercase, normalized) form.
///
/// Applies the same confusable normalization as [`decode_char`] without
/// reconstructing the underlying bytes — useful for non-byte-aligned fields
/// such as the Secret Key's account-id hint.
///
/// # Errors
///
/// Returns [`CryptoError::InvalidParams`] if any character is not a valid
/// Crockford symbol.
pub(crate) fn normalize(s: &str) -> Result<String, CryptoError> {
    s.chars()
        .map(|c| decode_char(c).map(|v| ALPHABET[v as usize] as char))
        .collect()
}

/// Insert dashes every `group_size` characters for readable display.
pub(crate) fn group(s: &str, group_size: usize) -> String {
    s.as_bytes()
        .chunks(group_size)
        .map(|chunk| {
            // SAFETY: callers pass ASCII-only Crockford output.
            std::str::from_utf8(chunk).expect("Crockford output is always valid UTF-8")
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Decode a single Crockford Base32 character to its 5-bit value.
///
/// Applies normalization: `O` → 0, `I`/`L` → 1. Rejects `U` (not in alphabet).
pub(crate) fn decode_char(c: char) -> Result<u8, CryptoError> {
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
            "invalid Crockford Base32 character: '{c}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_len_matches_recovery_and_secret_key() {
        assert_eq!(encoded_len(32), 52); // recovery key
        assert_eq!(encoded_len(16), 26); // secret key body
    }

    #[test]
    fn round_trip_arbitrary_lengths() {
        for len in [1usize, 4, 16, 32] {
            let data: Vec<u8> = (0..len)
                .map(|i| u8::try_from(i).unwrap().wrapping_mul(7).wrapping_add(1))
                .collect();
            let encoded = encode(&data);
            assert_eq!(encoded.len(), encoded_len(len));
            let decoded = decode(&encoded, len).unwrap();
            assert_eq!(decoded, data);
        }
    }

    #[test]
    fn decode_rejects_wrong_length() {
        assert!(decode("ABC", 16).is_err());
    }

    #[test]
    fn decode_rejects_invalid_char() {
        let mut s = encode(&[0u8; 16]);
        s.replace_range(0..1, "U"); // U is not in the alphabet
        assert!(decode(&s, 16).is_err());
    }

    #[test]
    fn normalize_maps_confusables() {
        assert_eq!(normalize("oIlO").unwrap(), "0110");
        assert_eq!(normalize("abc").unwrap(), "ABC");
        assert!(normalize("U").is_err());
    }

    #[test]
    fn group_inserts_dashes() {
        assert_eq!(group("ABCDEFGH", 4), "ABCD-EFGH");
        assert_eq!(group("ABCDE", 4), "ABCD-E");
    }
}
