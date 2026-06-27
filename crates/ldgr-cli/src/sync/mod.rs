//! Sync transport implementations for the CLI.
//!
//! Defines the async `BlobTransport` trait and concrete implementations
//! for Dropbox and `WebDAV`. Includes retry middleware.

#![allow(dead_code)] // Trait methods used by implementations; not all called yet

pub mod dropbox;
pub mod server;
pub mod webdav;

use std::fmt;

use ldgr_core::sync::transport::{BlobPath, BlobPrefix, ListResult, PutResult, TransportErrorKind};

// ── Transport Error ────────────────────────────────────────────────────────────

/// Transport error with kind classification for retry decisions.
#[derive(Debug)]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub message: String,
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl TransportError {
    pub fn new(kind: TransportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        kind: TransportErrorKind,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)?;
        if let Some(ref src) = self.source {
            write!(f, " (caused by: {src})")?;
        }
        Ok(())
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

// ── Blob Transport Trait ───────────────────────────────────────────────────────

/// Async blob storage transport.
///
/// Low-level blob primitives that each provider implements.
/// Sync orchestration logic composes these into push/pull workflows.
#[async_trait::async_trait]
pub trait BlobTransport: Send + Sync {
    /// Upload a blob. Uses put-if-absent semantics for immutable blobs
    /// (event batches, snapshots).
    async fn put_blob(&self, path: &BlobPath, data: &[u8]) -> Result<PutResult, TransportError>;

    /// Download a blob by path. Returns the raw bytes.
    async fn get_blob(&self, path: &BlobPath) -> Result<Vec<u8>, TransportError>;

    /// List blobs under a prefix, with optional pagination cursor.
    async fn list_blobs(
        &self,
        prefix: &BlobPrefix,
        cursor: Option<&str>,
    ) -> Result<ListResult, TransportError>;

    /// Delete a blob by path. Idempotent — returns Ok if already absent.
    async fn delete_blob(&self, path: &BlobPath) -> Result<(), TransportError>;

    /// Check if a blob exists.
    async fn exists(&self, path: &BlobPath) -> Result<bool, TransportError>;

    /// Ensure the directory structure exists for the given path.
    async fn ensure_directory(&self, prefix: &BlobPrefix) -> Result<(), TransportError>;
}

// ── Retry Middleware ───────────────────────────────────────────────────────────

/// Wraps a transport with retry logic.
pub struct RetryTransport<T: BlobTransport> {
    inner: T,
    policy: ldgr_core::sync::RetryPolicy,
}

impl<T: BlobTransport> RetryTransport<T> {
    pub fn new(inner: T, policy: ldgr_core::sync::RetryPolicy) -> Self {
        Self { inner, policy }
    }

    async fn with_retry<F, Fut, R>(&self, operation: &str, f: F) -> Result<R, TransportError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<R, TransportError>>,
    {
        let mut last_err = None;
        for attempt in 0..=self.policy.max_retries {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if !e.kind.is_retryable() || attempt == self.policy.max_retries {
                        return Err(e);
                    }
                    let backoff = self.policy.backoff_ms(attempt);
                    eprintln!(
                        "  ⟳ {operation} failed (attempt {}/{}): {}. Retrying in {backoff}ms…",
                        attempt + 1,
                        self.policy.max_retries + 1,
                        e.message,
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            TransportError::new(TransportErrorKind::Other, "retry exhausted with no error")
        }))
    }
}

#[async_trait::async_trait]
impl<T: BlobTransport> BlobTransport for RetryTransport<T> {
    async fn put_blob(&self, path: &BlobPath, data: &[u8]) -> Result<PutResult, TransportError> {
        self.with_retry("put_blob", || self.inner.put_blob(path, data))
            .await
    }

    async fn get_blob(&self, path: &BlobPath) -> Result<Vec<u8>, TransportError> {
        self.with_retry("get_blob", || self.inner.get_blob(path))
            .await
    }

    async fn list_blobs(
        &self,
        prefix: &BlobPrefix,
        cursor: Option<&str>,
    ) -> Result<ListResult, TransportError> {
        self.with_retry("list_blobs", || self.inner.list_blobs(prefix, cursor))
            .await
    }

    async fn delete_blob(&self, path: &BlobPath) -> Result<(), TransportError> {
        self.with_retry("delete_blob", || self.inner.delete_blob(path))
            .await
    }

    async fn exists(&self, path: &BlobPath) -> Result<bool, TransportError> {
        self.with_retry("exists", || self.inner.exists(path)).await
    }

    async fn ensure_directory(&self, prefix: &BlobPrefix) -> Result<(), TransportError> {
        self.with_retry("ensure_directory", || self.inner.ensure_directory(prefix))
            .await
    }
}
