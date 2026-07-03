//! Self-hosted `ldgr-server` blob transport.
//!
//! Bridges the CLI's path-keyed [`BlobTransport`] onto the endpoint-based
//! [`ServerSyncClient`] from `ldgr-core`. The core client does no I/O itself;
//! this module supplies the [`RawHttpSender`] seam over `reqwest` — the only
//! place in the workspace that performs HTTP for server sync.
//!
//! Routing (canonical blob layout → server endpoint):
//! ```text
//! {vault}/batches/{device}/{batch}.enc   → put_batch / get_batch
//! {vault}/snapshots/{snapshot}.enc       → put_snapshot / get_snapshot
//! {vault}/devices/{device}.json.enc      → put_device
//! ```
//!
//! Authentication is performed once during `ldgr sync setup` (SRP-6a login,
//! `&mut self`); the resulting bearer token is persisted and the transport is
//! rebuilt with [`ServerSyncClient::with_token`]. The password is never stored,
//! so there is no silent mid-sync re-login: an auth failure surfaces a clear
//! "re-run `ldgr sync setup`" message.

use std::time::Duration;

use ldgr_core::sync::server::{
    HttpMethod, ListBatchesQuery, ListSnapshotsQuery, RawHttpSender, RawRequest, RawResponse,
    ServerSyncClient, ServerSyncError,
};
use ldgr_core::sync::transport::{
    BlobEntry, BlobPath, BlobPrefix, ListResult, PutResult, TransportErrorKind, batches_prefix,
    parse_batch_path, parse_snapshot_path, snapshots_prefix,
};
use reqwest::Client;

use super::{BlobTransport, TransportError};

// ── Reqwest sender (the core HTTP seam) ─────────────────────────────────────────

/// Executes [`RawRequest`]s from the core client over `reqwest`.
pub struct ReqwestSender {
    client: Client,
    base_url: String,
}

impl ReqwestSender {
    /// Build a sender targeting `base_url` (e.g. `https://sync.example.com`).
    ///
    /// A trailing slash is trimmed so it composes cleanly with the absolute
    /// `/api/v1/...` paths the core client produces.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to create HTTP client");
        Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    fn method(method: HttpMethod) -> reqwest::Method {
        match method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Delete => reqwest::Method::DELETE,
        }
    }
}

impl RawHttpSender for ReqwestSender {
    async fn send(&self, request: RawRequest) -> Result<RawResponse, ServerSyncError> {
        let url = format!("{}{}", self.base_url, request.path);
        let mut builder = self.client.request(Self::method(request.method), url);

        if !request.query.is_empty() {
            builder = builder.query(&request.query);
        }
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| ServerSyncError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| ServerSyncError::Transport(e.to_string()))?
            .to_vec();

        Ok(RawResponse { status, body })
    }
}

// ── Server transport ────────────────────────────────────────────────────────────

/// A [`BlobTransport`] backed by an authenticated [`ServerSyncClient`].
pub struct ServerTransport {
    client: ServerSyncClient<ReqwestSender>,
    vault_id: String,
}

impl ServerTransport {
    /// Build an authenticated transport from a persisted session token.
    #[must_use]
    pub fn new(base_url: impl Into<String>, token: String, vault_id: String) -> Self {
        let sender = ReqwestSender::new(base_url);
        let client = ServerSyncClient::with_token(sender, token);
        Self { client, vault_id }
    }
}

/// Parse a device blob path `{vault}/devices/{device_id}.json.enc`.
fn parse_device_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 3 || parts[1] != "devices" {
        return None;
    }
    parts[2].strip_suffix(".json.enc").map(String::from)
}

/// Map a [`ServerSyncError`] into a [`TransportError`] so [`RetryTransport`]
/// keeps its retry classification working.
///
/// [`RetryTransport`]: super::RetryTransport
fn to_transport_error(err: ServerSyncError) -> TransportError {
    match err {
        ServerSyncError::Transport(msg) => TransportError::new(TransportErrorKind::Network, msg),
        ServerSyncError::Http { status, message } => {
            let kind = match status {
                401 | 403 => TransportErrorKind::Auth,
                404 => TransportErrorKind::NotFound,
                409 => TransportErrorKind::Conflict,
                413 => TransportErrorKind::PayloadTooLarge,
                429 => TransportErrorKind::RateLimited,
                s if (500..600).contains(&s) => TransportErrorKind::Server,
                _ => TransportErrorKind::Other,
            };
            TransportError::new(kind, format!("server returned {status}: {message}"))
        }
        ServerSyncError::NotAuthenticated => TransportError::new(
            TransportErrorKind::Auth,
            "session expired or missing — re-run `ldgr sync setup`",
        ),
        ServerSyncError::Decode(msg) => {
            TransportError::new(TransportErrorKind::InvalidResponse, msg)
        }
        other => TransportError::new(TransportErrorKind::Other, other.to_string()),
    }
}

fn unsupported(operation: &str, path: &str) -> TransportError {
    TransportError::new(
        TransportErrorKind::Other,
        format!("{operation} is not supported by the ldgr-server transport for path `{path}`"),
    )
}

#[async_trait::async_trait]
impl BlobTransport for ServerTransport {
    async fn put_blob(&self, path: &BlobPath, data: &[u8]) -> Result<PutResult, TransportError> {
        let p = path.as_str();
        let resp = if let Some(b) = parse_batch_path(p) {
            self.client
                .put_batch(&self.vault_id, &b.device_id, &b.batch_id, data)
                .await
        } else if let Some(snapshot_id) = parse_snapshot_path(p) {
            self.client
                .put_snapshot(&self.vault_id, &snapshot_id, data)
                .await
        } else if let Some(device_id) = parse_device_path(p) {
            // Device info is opaque to the server; there is no PutBlobResponse.
            return self
                .client
                .put_device(&self.vault_id, &device_id, data)
                .await
                .map(|()| PutResult {
                    content_hash: None,
                    etag: None,
                })
                .map_err(to_transport_error);
        } else {
            return Err(unsupported("put_blob", p));
        };

        resp.map(|r| PutResult {
            content_hash: Some(r.content_hash),
            etag: None,
        })
        .map_err(to_transport_error)
    }

    async fn get_blob(&self, path: &BlobPath) -> Result<Vec<u8>, TransportError> {
        let p = path.as_str();
        if let Some(b) = parse_batch_path(p) {
            self.client
                .get_batch(&self.vault_id, &b.device_id, &b.batch_id)
                .await
                .map_err(to_transport_error)
        } else if let Some(snapshot_id) = parse_snapshot_path(p) {
            self.client
                .get_snapshot(&self.vault_id, &snapshot_id)
                .await
                .map_err(to_transport_error)
        } else {
            Err(unsupported("get_blob", p))
        }
    }

    async fn list_blobs(
        &self,
        prefix: &BlobPrefix,
        _cursor: Option<&str>,
    ) -> Result<ListResult, TransportError> {
        let pfx = prefix.as_str();

        // The server pages results and returns an opaque keyset cursor (over
        // `(created_at, path)`) via the `has_more`/`cursor` fields. Follow it
        // until exhausted so vaults with more blobs than a single server page
        // (~1000) list — and therefore pull — completely. Entries are collated
        // across pages and surfaced as one logical listing.
        if pfx == batches_prefix(&self.vault_id).as_str() {
            let mut entries = Vec::new();
            let mut query = ListBatchesQuery::default();
            loop {
                let page = self
                    .client
                    .list_remote_batches_page(&self.vault_id, &query)
                    .await
                    .map_err(to_transport_error)?;
                entries.extend(page.metas.into_iter().map(|m| BlobEntry {
                    path: m.path,
                    size: m.size,
                    content_hash: m.content_hash,
                    modified_at: m.modified_at,
                }));
                match page.next_cursor {
                    Some(cursor) if page.has_more => query.since = Some(cursor),
                    _ => break,
                }
            }
            Ok(ListResult {
                entries,
                cursor: None,
                has_more: false,
            })
        } else if pfx == snapshots_prefix(&self.vault_id).as_str() {
            let mut entries = Vec::new();
            let mut query = ListSnapshotsQuery::default();
            loop {
                let page = self
                    .client
                    .list_remote_snapshots_page(&self.vault_id, &query)
                    .await
                    .map_err(to_transport_error)?;
                entries.extend(page.metas.into_iter().map(|m| BlobEntry {
                    path: m.path,
                    size: m.size,
                    content_hash: m.content_hash,
                    modified_at: m.modified_at,
                }));
                match page.next_cursor {
                    Some(cursor) if page.has_more => query.since = Some(cursor),
                    _ => break,
                }
            }
            Ok(ListResult {
                entries,
                cursor: None,
                has_more: false,
            })
        } else {
            // Other prefixes (e.g. per-device batch prefixes) aren't listed by
            // the engine; return an empty page rather than erroring.
            Ok(ListResult {
                entries: Vec::new(),
                cursor: None,
                has_more: false,
            })
        }
    }

    async fn delete_blob(&self, path: &BlobPath) -> Result<(), TransportError> {
        // The server exposes no batch/snapshot delete endpoint (immutable
        // blobs); the push/pull engine never calls this.
        Err(unsupported("delete_blob", path.as_str()))
    }

    async fn exists(&self, path: &BlobPath) -> Result<bool, TransportError> {
        match self.get_blob(path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind == TransportErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn ensure_directory(&self, _prefix: &BlobPrefix) -> Result<(), TransportError> {
        // The server has no directory concept — blobs are keyed by full path.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_path() {
        assert_eq!(
            parse_device_path("vault_1/devices/dev_9.json.enc").as_deref(),
            Some("dev_9")
        );
        assert!(parse_device_path("vault_1/batches/dev_9/b.enc").is_none());
        assert!(parse_device_path("vault_1/devices/dev_9.enc").is_none());
    }

    #[test]
    fn classifies_http_status() {
        let auth = to_transport_error(ServerSyncError::Http {
            status: 401,
            message: "no".into(),
        });
        assert_eq!(auth.kind, TransportErrorKind::Auth);

        let conflict = to_transport_error(ServerSyncError::Http {
            status: 409,
            message: "dup".into(),
        });
        assert_eq!(conflict.kind, TransportErrorKind::Conflict);

        let not_found = to_transport_error(ServerSyncError::Http {
            status: 404,
            message: "gone".into(),
        });
        assert_eq!(not_found.kind, TransportErrorKind::NotFound);

        let server = to_transport_error(ServerSyncError::Http {
            status: 503,
            message: "down".into(),
        });
        assert_eq!(server.kind, TransportErrorKind::Server);
    }

    #[test]
    fn classifies_transport_as_network() {
        let err = to_transport_error(ServerSyncError::Transport("dns".into()));
        assert_eq!(err.kind, TransportErrorKind::Network);
        assert!(err.kind.is_retryable());
    }
}
