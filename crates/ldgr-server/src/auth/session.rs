//! Session token generation and hashing.
//!
//! Tokens are random 256-bit values. The server stores only the SHA-256
//! hash of the token, so a database leak does not compromise sessions.

use rand::RngCore;
use sha2::{Digest, Sha256};

use super::{hex_decode, hex_encode};
use crate::error::ServerError;

/// Generate a new session token. Returns `(raw_token_hex, token_hash_hex)`.
///
/// The raw token is sent to the client once. The hash is stored in the database.
pub fn generate_token() -> (String, String) {
    let mut token_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut token_bytes);
    let raw_hex = hex_encode(&token_bytes);
    let hash_hex = hash_token_bytes(&token_bytes);
    (raw_hex, hash_hex)
}

/// Hash a raw token (hex-encoded) to produce the stored hash.
pub fn hash_token_hex(token_hex: &str) -> Result<String, ServerError> {
    let bytes = hex_decode(token_hex)
        .map_err(|e| ServerError::BadRequest(format!("invalid token: {e}")))?;
    Ok(hash_token_bytes(&bytes))
}

fn hash_token_bytes(token: &[u8]) -> String {
    let digest = Sha256::digest(token);
    hex_encode(&digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_generation() {
        let (raw, hash) = generate_token();
        assert_eq!(raw.len(), 64); // 32 bytes × 2 hex chars
        assert_eq!(hash.len(), 64); // SHA-256 = 32 bytes × 2 hex chars
        assert_ne!(raw, hash);
    }

    #[test]
    fn hash_is_deterministic() {
        let (raw, hash) = generate_token();
        let hash2 = hash_token_hex(&raw).unwrap();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn hash_invalid_token() {
        assert!(hash_token_hex("not-hex").is_err());
    }
}
