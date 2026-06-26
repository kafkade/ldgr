//! Serde request/response types for every `ldgr-server` endpoint.
//!
//! These mirror the structs in `crates/ldgr-server/src/api/*` field-for-field
//! so that they round-trip across the wire. Binary values that the server
//! encodes as hex strings (salt, verifier, public values, proofs, encrypted
//! device/relay payloads) are represented here as `String` and converted with
//! the [`hex_encode`]/[`hex_decode`] helpers. Raw blob bodies (batches,
//! snapshots, device info) are transferred as `Vec<u8>` octet-streams, not JSON.
//!
//! Pure data — no I/O.

use serde::{Deserialize, Serialize};

// ── Hex helpers ────────────────────────────────────────────────────────────────

/// Encode bytes as a lowercase hex string (matches the server's `hex_encode`).
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Decode a hex string into bytes (matches the server's `hex_decode`).
///
/// # Errors
///
/// Returns an error if the string has an odd length or contains a non-hex
/// character.
pub fn hex_decode(hex: &str) -> Result<Vec<u8>, HexError> {
    if !hex.len().is_multiple_of(2) {
        return Err(HexError::OddLength);
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| HexError::InvalidChar))
        .collect()
}

/// Error decoding a hex string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HexError {
    /// The hex string has an odd number of characters.
    #[error("hex string has odd length")]
    OddLength,
    /// The hex string contains a non-hex character.
    #[error("invalid hex character")]
    InvalidChar,
}

// ── Auth: Register ──────────────────────────────────────────────────────────────

/// `POST /api/v1/auth/register` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    /// Hex-encoded salt.
    pub salt: String,
    /// Hex-encoded SRP verifier.
    pub verifier: String,
}

/// `POST /api/v1/auth/register` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub user_id: String,
}

// ── Auth: Login init (SRP step 1) ───────────────────────────────────────────────

/// `POST /api/v1/auth/login/init` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginInitRequest {
    pub username: String,
    /// Hex-encoded client public value A.
    pub client_public: String,
}

/// `POST /api/v1/auth/login/init` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginInitResponse {
    pub handshake_id: String,
    /// Hex-encoded salt.
    pub salt: String,
    /// Hex-encoded server public value B.
    pub server_public: String,
}

// ── Auth: Login verify (SRP step 2) ─────────────────────────────────────────────

/// `POST /api/v1/auth/login/verify` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginVerifyRequest {
    pub handshake_id: String,
    /// Hex-encoded client proof M1.
    pub client_proof: String,
}

/// `POST /api/v1/auth/login/verify` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginVerifyResponse {
    /// Hex-encoded server proof M2.
    pub server_proof: String,
    /// Bearer session token.
    pub token: String,
}

// ── Vaults ──────────────────────────────────────────────────────────────────────

/// `POST /api/v1/vaults` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVaultRequest {
    pub vault_id: String,
}

/// Vault descriptor returned by create/list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultResponse {
    pub id: String,
    pub created_at: String,
}

// ── Blobs (batches & snapshots) ─────────────────────────────────────────────────

/// Response to a blob `PUT` (batch or snapshot upload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutBlobResponse {
    pub path: String,
    pub size: i64,
    pub content_hash: String,
}

/// A single entry in a blob listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobEntry {
    pub path: String,
    pub size: i64,
    pub content_hash: String,
    pub created_at: String,
}

/// Response to a blob list request (batches or snapshots).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListBlobsResponse {
    pub entries: Vec<BlobEntry>,
    pub has_more: bool,
}

/// Query parameters for `GET /api/v1/vaults/:vault_id/batches`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListBatchesQuery {
    pub since: Option<String>,
    pub device_id: Option<String>,
    pub limit: Option<u32>,
}

/// Query parameters for `GET /api/v1/vaults/:vault_id/snapshots`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListSnapshotsQuery {
    pub since: Option<String>,
    pub limit: Option<u32>,
}

// ── Devices ─────────────────────────────────────────────────────────────────────

/// Device descriptor returned by `GET /api/v1/vaults/:vault_id/devices`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceResponse {
    pub id: String,
    pub updated_at: String,
    /// Hex-encoded encrypted device info (opaque to the server).
    pub encrypted_info: String,
}

// ── Relay ───────────────────────────────────────────────────────────────────────

/// `POST /api/v1/relay/offer` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOfferRequest {
    /// Hex-encoded encrypted offer payload.
    pub offer_data: String,
}

/// `POST /api/v1/relay/offer` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOfferResponse {
    pub offer_id: String,
    pub expires_at: String,
}

/// `GET /api/v1/relay/:offer_id` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferResponse {
    pub offer_id: String,
    /// Hex-encoded encrypted offer payload.
    pub offer_data: String,
    pub expires_at: String,
    pub has_response: bool,
}

/// `POST /api/v1/relay/:offer_id/respond` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostResponseRequest {
    /// Hex-encoded encrypted response payload.
    pub response_data: String,
}

/// `GET /api/v1/relay/:offer_id/response` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetResponseResponse {
    /// Hex-encoded encrypted response payload.
    pub response_data: String,
}

/// Error envelope returned by the server for non-2xx responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip through serde and assert the re-serialized form is identical.
    /// Avoids requiring `PartialEq` on every protocol type.
    fn round_trip<T>(value: &T)
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2);
    }

    #[test]
    fn hex_round_trip() {
        let data = b"\x00\x01\xfe\xff hello";
        let encoded = hex_encode(data);
        assert_eq!(&encoded[..8], "0001feff");
        assert_eq!(hex_decode(&encoded).unwrap(), data);
    }

    #[test]
    fn hex_decode_errors() {
        assert_eq!(hex_decode("abc"), Err(HexError::OddLength));
        assert_eq!(hex_decode("0g"), Err(HexError::InvalidChar));
    }

    #[test]
    fn auth_types_round_trip() {
        round_trip(&RegisterRequest {
            username: "alice".into(),
            salt: "00ff".into(),
            verifier: "abcd".into(),
        });
        round_trip(&RegisterResponse {
            user_id: "u1".into(),
        });
        round_trip(&LoginInitRequest {
            username: "alice".into(),
            client_public: "aa".into(),
        });
        round_trip(&LoginInitResponse {
            handshake_id: "h1".into(),
            salt: "00ff".into(),
            server_public: "bb".into(),
        });
        round_trip(&LoginVerifyRequest {
            handshake_id: "h1".into(),
            client_proof: "cc".into(),
        });
        round_trip(&LoginVerifyResponse {
            server_proof: "dd".into(),
            token: "tok".into(),
        });
    }

    #[test]
    fn vault_and_blob_types_round_trip() {
        round_trip(&CreateVaultRequest {
            vault_id: "v1".into(),
        });
        round_trip(&VaultResponse {
            id: "v1".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        });
        round_trip(&PutBlobResponse {
            path: "v1/batches/d1/b1.enc".into(),
            size: 42,
            content_hash: "deadbeef".into(),
        });
        round_trip(&ListBlobsResponse {
            entries: vec![BlobEntry {
                path: "v1/snapshots/s1.enc".into(),
                size: 7,
                content_hash: "00".into(),
                created_at: "t".into(),
            }],
            has_more: true,
        });
        round_trip(&ListBatchesQuery {
            since: Some("t".into()),
            device_id: Some("d1".into()),
            limit: Some(50),
        });
        round_trip(&ListSnapshotsQuery {
            since: None,
            limit: None,
        });
    }

    #[test]
    fn device_and_relay_types_round_trip() {
        round_trip(&DeviceResponse {
            id: "d1".into(),
            updated_at: "t".into(),
            encrypted_info: "00ff".into(),
        });
        round_trip(&CreateOfferRequest {
            offer_data: "00ff".into(),
        });
        round_trip(&CreateOfferResponse {
            offer_id: "o1".into(),
            expires_at: "t".into(),
        });
        round_trip(&OfferResponse {
            offer_id: "o1".into(),
            offer_data: "00ff".into(),
            expires_at: "t".into(),
            has_response: false,
        });
        round_trip(&PostResponseRequest {
            response_data: "00ff".into(),
        });
        round_trip(&GetResponseResponse {
            response_data: "00ff".into(),
        });
        round_trip(&ErrorResponse {
            error: "boom".into(),
        });
    }
}
