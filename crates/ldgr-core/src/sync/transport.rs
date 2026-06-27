//! Sync transport types and blob path layout.
//!
//! Pure computation — defines types and path helpers for sync transports.
//! No I/O, no networking. Platform code (CLI/iOS/web) implements the actual
//! transport using these types.
//!
//! The blob store layout is canonical across all transports:
//! ```text
//! {vault_id}/
//!   batches/{device_id}/{batch_id}.enc
//!   snapshots/{snapshot_id}.enc
//!   devices/{device_id}.json.enc
//! ```

use serde::{Deserialize, Serialize};

use super::events::VectorClock;

// ── Blob Path Layout ───────────────────────────────────────────────────────────

/// A path to a blob in the sync store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobPath(pub String);

impl BlobPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BlobPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Prefix for listing blobs.
#[derive(Debug, Clone)]
pub struct BlobPrefix(pub String);

impl BlobPrefix {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Compute the blob path for an event batch.
pub fn batch_path(vault_id: &str, device_id: &str, batch_id: &str) -> BlobPath {
    BlobPath(format!("{vault_id}/batches/{device_id}/{batch_id}.enc"))
}

/// Compute the blob path for a snapshot.
pub fn snapshot_path(vault_id: &str, snapshot_id: &str) -> BlobPath {
    BlobPath(format!("{vault_id}/snapshots/{snapshot_id}.enc"))
}

/// Compute the blob path for a device info file.
pub fn device_path(vault_id: &str, device_id: &str) -> BlobPath {
    BlobPath(format!("{vault_id}/devices/{device_id}.json.enc"))
}

/// Compute the prefix for listing all batches.
pub fn batches_prefix(vault_id: &str) -> BlobPrefix {
    BlobPrefix(format!("{vault_id}/batches/"))
}

/// Compute the prefix for listing all batches from a specific device.
pub fn device_batches_prefix(vault_id: &str, device_id: &str) -> BlobPrefix {
    BlobPrefix(format!("{vault_id}/batches/{device_id}/"))
}

/// Compute the prefix for listing all snapshots.
pub fn snapshots_prefix(vault_id: &str) -> BlobPrefix {
    BlobPrefix(format!("{vault_id}/snapshots/"))
}

/// Compute the prefix for listing all devices.
pub fn devices_prefix(vault_id: &str) -> BlobPrefix {
    BlobPrefix(format!("{vault_id}/devices/"))
}

/// Parse a remote batch path into its components.
#[derive(Debug, Clone)]
pub struct BatchRef {
    pub vault_id: String,
    pub device_id: String,
    pub batch_id: String,
}

/// Parse a blob path that matches the batch layout.
pub fn parse_batch_path(path: &str) -> Option<BatchRef> {
    // Expected: {vault_id}/batches/{device_id}/{batch_id}.enc
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 4 || parts[1] != "batches" {
        return None;
    }
    let batch_id = parts[3].strip_suffix(".enc")?;
    Some(BatchRef {
        vault_id: parts[0].to_string(),
        device_id: parts[2].to_string(),
        batch_id: batch_id.to_string(),
    })
}

/// Parse a blob path that matches the snapshot layout.
pub fn parse_snapshot_path(path: &str) -> Option<String> {
    // Expected: {vault_id}/snapshots/{snapshot_id}.enc
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 3 || parts[1] != "snapshots" {
        return None;
    }
    parts[2].strip_suffix(".enc").map(String::from)
}

// ── Metadata Types ─────────────────────────────────────────────────────────────

/// Metadata about a remote event batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteBatchMeta {
    pub batch_id: String,
    pub device_id: String,
    pub path: String,
    pub size: u64,
    pub content_hash: Option<String>,
    pub modified_at: Option<String>,
}

/// Metadata about a remote snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSnapshotMeta {
    pub snapshot_id: String,
    pub path: String,
    pub size: u64,
    pub content_hash: Option<String>,
    pub modified_at: Option<String>,
}

/// Information about a registered device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub last_sync_at: Option<String>,
    pub vector_clock: VectorClock,
}

// ── Transport Configuration ────────────────────────────────────────────────────

/// Transport provider type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportProvider {
    Dropbox,
    WebDav,
    Server,
}

impl TransportProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dropbox => "dropbox",
            Self::WebDav => "webdav",
            Self::Server => "server",
        }
    }
}

/// Transport configuration (non-secret data only).
///
/// Secrets (OAuth tokens, passwords) are stored separately in the
/// platform's secure credential store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransportConfig {
    Dropbox {
        /// Application key for OAuth.
        app_key: String,
        /// Hint for the account (email), not secret.
        account_hint: Option<String>,
    },
    WebDav {
        /// WebDAV server base URL.
        #[allow(clippy::doc_markdown)]
        base_url: String,
        /// Username hint (not the password).
        username: Option<String>,
    },
    /// Self-hosted `ldgr-server` (SRP-6a).
    ///
    /// Non-secret data only. The SRP session token is a bearer **secret** and is
    /// stored separately in the platform credential store (the CLI's
    /// `sync-credentials.json`), never in this config.
    Server {
        /// Base URL of the `ldgr-server` instance.
        #[allow(clippy::doc_markdown)]
        base_url: String,
        /// Account username hint (email), not secret.
        username: Option<String>,
        /// Remote vault identifier.
        vault_id: String,
        /// This device's identifier.
        device_id: String,
    },
}

impl TransportConfig {
    pub fn provider(&self) -> TransportProvider {
        match self {
            Self::Dropbox { .. } => TransportProvider::Dropbox,
            Self::WebDav { .. } => TransportProvider::WebDav,
            Self::Server { .. } => TransportProvider::Server,
        }
    }
}

// ── Error Types ────────────────────────────────────────────────────────────────

/// Sync transport error categories.
///
/// Used to classify errors for retry decisions. Platform code maps
/// provider-specific errors into these categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    /// Network connectivity issue (retryable).
    Network,
    /// Authentication failure (not retryable without re-auth).
    Auth,
    /// Blob not found.
    NotFound,
    /// Conflict (`ETag` mismatch, concurrent write).
    Conflict,
    /// Rate limited (retryable after backoff).
    RateLimited,
    /// Server error (retryable).
    Server,
    /// Invalid response from server.
    InvalidResponse,
    /// Request too large.
    PayloadTooLarge,
    /// Provider-specific error.
    Other,
}

impl TransportErrorKind {
    /// Whether this error category is retryable.
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Network | Self::RateLimited | Self::Server)
    }

    /// Human-readable label for this error category.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network error",
            Self::Auth => "authentication error",
            Self::NotFound => "not found",
            Self::Conflict => "conflict",
            Self::RateLimited => "rate limited",
            Self::Server => "server error",
            Self::InvalidResponse => "invalid response",
            Self::PayloadTooLarge => "payload too large",
            Self::Other => "transport error",
        }
    }
}

/// Retry policy for transport operations.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Initial backoff duration in milliseconds.
    pub initial_backoff_ms: u64,
    /// Maximum backoff duration in milliseconds.
    pub max_backoff_ms: u64,
    /// Backoff multiplier (exponential backoff).
    pub backoff_multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 500,
            max_backoff_ms: 30_000,
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryPolicy {
    /// Compute the backoff duration for a given attempt (0-indexed).
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    pub fn backoff_ms(&self, attempt: u32) -> u64 {
        let backoff = self.initial_backoff_ms as f64 * self.backoff_multiplier.powi(attempt as i32);
        (backoff as u64).min(self.max_backoff_ms)
    }
}

// ── Sync Checkpoint ────────────────────────────────────────────────────────────

/// Local sync checkpoint tracking what has been synced.
///
/// Stored locally per device — NOT synced to the remote store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncCheckpoint {
    /// Batch IDs that have been successfully synced (uploaded or downloaded).
    pub synced_batch_ids: Vec<String>,
    /// The vector clock at the last successful sync.
    pub vector_clock: VectorClock,
    /// Timestamp of the last successful sync.
    pub last_sync_at: Option<String>,
    /// Provider-specific opaque cursor (e.g., Dropbox cursor string).
    /// Used to optimize listing but NOT trusted for correctness.
    pub provider_cursor: Option<String>,
}

// ── Result Types ───────────────────────────────────────────────────────────────

/// Result of listing blobs.
#[derive(Debug, Clone)]
pub struct ListResult {
    /// Blob paths found.
    pub entries: Vec<BlobEntry>,
    /// Opaque cursor for pagination (provider-specific).
    pub cursor: Option<String>,
    /// Whether more results are available.
    pub has_more: bool,
}

/// A single entry from a blob listing.
#[derive(Debug, Clone)]
pub struct BlobEntry {
    pub path: String,
    pub size: u64,
    pub content_hash: Option<String>,
    pub modified_at: Option<String>,
}

/// Result of uploading a blob.
#[derive(Debug, Clone)]
pub struct PutResult {
    /// Content hash assigned by the provider.
    pub content_hash: Option<String>,
    /// `ETag` or revision for conditional operations.
    pub etag: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_path_format() {
        let path = batch_path("vault_abc", "dev_1", "batch_42");
        assert_eq!(path.as_str(), "vault_abc/batches/dev_1/batch_42.enc");
    }

    #[test]
    fn snapshot_path_format() {
        let path = snapshot_path("vault_abc", "snap_1");
        assert_eq!(path.as_str(), "vault_abc/snapshots/snap_1.enc");
    }

    #[test]
    fn device_path_format() {
        let path = device_path("vault_abc", "dev_1");
        assert_eq!(path.as_str(), "vault_abc/devices/dev_1.json.enc");
    }

    #[test]
    fn parse_valid_batch_path() {
        let parsed = parse_batch_path("vault_abc/batches/dev_1/batch_42.enc").unwrap();
        assert_eq!(parsed.vault_id, "vault_abc");
        assert_eq!(parsed.device_id, "dev_1");
        assert_eq!(parsed.batch_id, "batch_42");
    }

    #[test]
    fn parse_invalid_batch_path() {
        assert!(parse_batch_path("not/a/valid/batch/path.enc").is_none());
        assert!(parse_batch_path("vault/batches/dev/batch.txt").is_none());
        assert!(parse_batch_path("").is_none());
    }

    #[test]
    fn parse_valid_snapshot_path() {
        let id = parse_snapshot_path("vault_abc/snapshots/snap_1.enc").unwrap();
        assert_eq!(id, "snap_1");
    }

    #[test]
    fn parse_invalid_snapshot_path() {
        assert!(parse_snapshot_path("vault/batches/snap.enc").is_none());
    }

    #[test]
    fn retry_policy_backoff() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.backoff_ms(0), 500);
        assert_eq!(policy.backoff_ms(1), 1_000);
        assert_eq!(policy.backoff_ms(2), 2_000);
    }

    #[test]
    fn retry_policy_max_backoff() {
        let policy = RetryPolicy {
            max_backoff_ms: 1_000,
            ..Default::default()
        };
        assert_eq!(policy.backoff_ms(10), 1_000); // capped
    }

    #[test]
    fn error_kind_retryable() {
        assert!(TransportErrorKind::Network.is_retryable());
        assert!(TransportErrorKind::RateLimited.is_retryable());
        assert!(TransportErrorKind::Server.is_retryable());
        assert!(!TransportErrorKind::Auth.is_retryable());
        assert!(!TransportErrorKind::NotFound.is_retryable());
        assert!(!TransportErrorKind::Conflict.is_retryable());
    }

    #[test]
    fn transport_config_provider() {
        let cfg = TransportConfig::Dropbox {
            app_key: "key".into(),
            account_hint: None,
        };
        assert_eq!(cfg.provider(), TransportProvider::Dropbox);

        let cfg = TransportConfig::WebDav {
            base_url: "https://dav.example.com".into(),
            username: None,
        };
        assert_eq!(cfg.provider(), TransportProvider::WebDav);
    }

    #[test]
    fn transport_config_server_provider_and_round_trip() {
        let cfg = TransportConfig::Server {
            base_url: "https://sync.example.com".into(),
            username: Some("alice@example.com".into()),
            vault_id: "vault_1".into(),
            device_id: "dev_1".into(),
        };
        assert_eq!(cfg.provider(), TransportProvider::Server);
        assert_eq!(cfg.provider().as_str(), "server");

        // Round-trips through serde unchanged.
        let json = serde_json::to_string(&cfg).expect("serialize");
        let back: TransportConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.provider(), TransportProvider::Server);

        // The session token is a secret and must never be part of this config.
        assert!(
            !json.contains("token"),
            "config must not carry a session token: {json}"
        );

        // Existing externally-tagged configs still parse identically (adding a
        // variant must not break backward compatibility).
        let legacy = r#"{"Dropbox":{"app_key":"k","account_hint":null}}"#;
        assert_eq!(
            serde_json::from_str::<TransportConfig>(legacy)
                .unwrap()
                .provider(),
            TransportProvider::Dropbox
        );
    }

    #[test]
    fn prefixes() {
        assert_eq!(batches_prefix("v1").as_str(), "v1/batches/");
        assert_eq!(device_batches_prefix("v1", "d1").as_str(), "v1/batches/d1/");
        assert_eq!(snapshots_prefix("v1").as_str(), "v1/snapshots/");
        assert_eq!(devices_prefix("v1").as_str(), "v1/devices/");
    }
}
