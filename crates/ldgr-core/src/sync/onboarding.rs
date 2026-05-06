//! Device onboarding via X25519 key exchange.
//!
//! Pure computation — generates keypairs and computes shared secrets.
//! Platform code handles QR display/scanning and network transport.
//!
//! Flow:
//! 1. Existing device generates ephemeral X25519 keypair
//! 2. Encodes public key + connection info into QR payload
//! 3. New device scans QR, generates its own keypair
//! 4. Both derive shared secret via X25519 Diffie-Hellman
//! 5. Existing device encrypts vault key with shared secret → sends
//! 6. New device decrypts vault key → onboarding complete

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use x25519_dalek::{EphemeralSecret, PublicKey};

/// Data encoded in the QR code by the existing device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QrPayload {
    /// X25519 public key (32 bytes, base64-encoded).
    pub public_key: String,
    /// Connection info (LAN IP:port or relay server URL).
    pub connection: String,
    /// Short verification code for MITM prevention.
    pub verification_code: String,
}

/// Result of initiating onboarding on the existing device.
pub struct OnboardingInitiation {
    /// The secret key (kept in memory, never transmitted).
    pub secret: EphemeralSecret,
    /// The QR payload to display.
    pub qr_payload: QrPayload,
}

/// Generate an ephemeral keypair and QR payload for device onboarding.
///
/// The `connection` string should be the LAN address or relay URL
/// where the new device can connect.
pub fn initiate_onboarding(connection: &str) -> OnboardingInitiation {
    let secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let public = PublicKey::from(&secret);

    let verification_code = generate_verification_code(&public);

    let qr_payload = QrPayload {
        public_key: base64_encode(public.as_bytes()),
        connection: connection.to_string(),
        verification_code,
    };

    OnboardingInitiation { secret, qr_payload }
}

/// Complete the key exchange on the new device side.
///
/// Takes the scanned QR payload and returns the new device's public key
/// and the shared secret.
pub struct OnboardingResponse {
    /// New device's public key to send back.
    pub public_key_b64: String,
    /// Derived shared secret (32 bytes) for encrypting the vault key.
    pub shared_secret: [u8; 32],
    /// Verification code (must match the one shown by existing device).
    pub verification_code: String,
}

/// Generate the new device's side of the key exchange.
pub fn respond_to_onboarding(qr_payload: &QrPayload) -> Result<OnboardingResponse, String> {
    let their_public_bytes = base64_decode(&qr_payload.public_key)?;
    let their_public: [u8; 32] = their_public_bytes
        .try_into()
        .map_err(|_| "invalid public key length")?;
    let their_public = PublicKey::from(their_public);

    let our_secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let our_public = PublicKey::from(&our_secret);

    let shared = our_secret.diffie_hellman(&their_public);

    Ok(OnboardingResponse {
        public_key_b64: base64_encode(our_public.as_bytes()),
        shared_secret: *shared.as_bytes(),
        verification_code: qr_payload.verification_code.clone(),
    })
}

/// Complete the key exchange on the existing device side.
///
/// Takes the existing device's secret and the new device's public key,
/// returns the shared secret.
pub fn complete_onboarding(
    secret: EphemeralSecret,
    new_device_public_b64: &str,
) -> Result<[u8; 32], String> {
    let their_public_bytes = base64_decode(new_device_public_b64)?;
    let their_public: [u8; 32] = their_public_bytes
        .try_into()
        .map_err(|_| "invalid public key length")?;
    let their_public = PublicKey::from(their_public);

    let shared = secret.diffie_hellman(&their_public);
    Ok(*shared.as_bytes())
}

/// Encrypt the vault key with the shared secret for transmission.
pub fn encrypt_vault_key(
    shared_secret: &[u8; 32],
    vault_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    let cipher =
        Aes256Gcm::new_from_slice(shared_secret).map_err(|e| format!("cipher error: {e}"))?;

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, vault_key.as_ref())
        .map_err(|_| "encryption failed".to_string())?;

    // Prepend nonce to ciphertext
    let mut result = Vec::with_capacity(12 + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypt the vault key received during onboarding.
pub fn decrypt_vault_key(shared_secret: &[u8; 32], encrypted: &[u8]) -> Result<[u8; 32], String> {
    if encrypted.len() < 12 {
        return Err("encrypted data too short".into());
    }

    let nonce = Nonce::from_slice(&encrypted[..12]);
    let ciphertext = &encrypted[12..];

    let cipher =
        Aes256Gcm::new_from_slice(shared_secret).map_err(|e| format!("cipher error: {e}"))?;

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "decryption failed (wrong shared secret or corrupted data)".to_string())?;

    let mut key = [0u8; 32];
    if plaintext.len() != 32 {
        return Err("decrypted vault key has wrong length".into());
    }
    key.copy_from_slice(&plaintext);
    Ok(key)
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Generate a 6-digit verification code from a public key.
fn generate_verification_code(public_key: &PublicKey) -> String {
    let bytes = public_key.as_bytes();
    // Simple hash: sum of bytes mod 1_000_000
    let sum: u64 = bytes.iter().map(|&b| u64::from(b)).sum();
    format!("{:06}", sum % 1_000_000)
}

fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        let _ = write!(result, "{}", CHARS[((n >> 18) & 63) as usize] as char);
        let _ = write!(result, "{}", CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            let _ = write!(result, "{}", CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            let _ = write!(result, "{}", CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim_end_matches('=');
    let mut result = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits = 0;

    for ch in s.chars() {
        let val = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 26,
            '0'..='9' => ch as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => return Err(format!("invalid base64 character: '{ch}'")),
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            #[allow(clippy::cast_possible_truncation)]
            result.push((buf >> bits) as u8);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initiation_produces_valid_qr() {
        let init = initiate_onboarding("192.168.1.100:8080");
        assert!(!init.qr_payload.public_key.is_empty());
        assert_eq!(init.qr_payload.connection, "192.168.1.100:8080");
        assert_eq!(init.qr_payload.verification_code.len(), 6);
    }

    #[test]
    fn vault_key_encrypt_decrypt_round_trip() {
        let shared_secret = [0xAA; 32];
        let vault_key = [0xBB; 32];

        let encrypted = encrypt_vault_key(&shared_secret, &vault_key).unwrap();
        let decrypted = decrypt_vault_key(&shared_secret, &encrypted).unwrap();
        assert_eq!(decrypted, vault_key);
    }

    #[test]
    fn wrong_shared_secret_fails_decrypt() {
        let shared1 = [0xAA; 32];
        let shared2 = [0xCC; 32];
        let vault_key = [0xBB; 32];

        let encrypted = encrypt_vault_key(&shared1, &vault_key).unwrap();
        assert!(decrypt_vault_key(&shared2, &encrypted).is_err());
    }

    #[test]
    fn base64_round_trip() {
        let data = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32,
        ];
        let encoded = base64_encode(&data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn verification_code_six_digits() {
        let init = initiate_onboarding("test");
        assert!(init.qr_payload.verification_code.len() == 6);
        assert!(
            init.qr_payload
                .verification_code
                .chars()
                .all(|c| c.is_ascii_digit())
        );
    }

    #[test]
    fn qr_payload_serializes() {
        let init = initiate_onboarding("192.168.1.1:9000");
        let json = serde_json::to_string(&init.qr_payload).unwrap();
        let parsed: QrPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.connection, "192.168.1.1:9000");
    }
}
