//! Server-sync FFI surface (issue #200).
//!
//! Exposes `ldgr-core`'s transport-agnostic
//! [`ServerSyncClient`](ldgr_core::sync::server::ServerSyncClient) to host
//! platforms (Swift/URLSession, and later web/`fetch` via a separate
//! `ldgr-wasm` crate) by injecting a **foreign async HTTP sender**. The host
//! performs the actual network I/O; all protocol logic (SRP-6a auth, endpoint
//! routing, batch/snapshot/device blob handling) stays in `ldgr-core`, which
//! remains zero-I/O.
//!
//! ## What crosses the boundary
//!
//! Only opaque bytes: request/response bodies are `Vec<u8>` and blob
//! payloads (`put_batch`/`get_batch`, snapshots, device info) are **ciphertext**
//! produced/consumed by the vault crypto layer. Plaintext financial data never
//! crosses the FFI in the clear.
//!
//! ## Auth
//!
//! Single-secret SRP via [`LdgrSyncClient::login`], and two-secret (2SKD,
//! ADR-008) via [`LdgrSyncClient::login_2skd`] /
//! [`LdgrSyncClient::register_2skd`]. The 2SKD methods take the **human inputs**
//! (password bytes + Secret Key text + the account's Argon2 salt/params) and
//! derive `MK_auth` + parse the [`SecretKey`] *inside Rust* — raw key material
//! never crosses the boundary.
//!
//! NOTE: provisioning the Argon2 salt/params onto a brand-new device
//! (Emergency-Kit / QR onboarding) is owned by the onboarding work (#181/#175)
//! and is intentionally out of scope here; callers on an existing device read
//! these from the local vault header.

use std::sync::Arc;

use futures::lock::Mutex;
use uuid::Uuid;

use ldgr_core::crypto::{Argon2Params, SecretKey, derive_auth_key, derive_master_key};
use ldgr_core::sync::server::{
    HttpMethod, ListBatchesQuery, ListSnapshotsQuery, RawHttpSender, RawRequest, RawResponse,
    ServerSyncClient, ServerSyncError,
};

// ── Error ────────────────────────────────────────────────────────────────────

/// Errors surfaced by the server-sync FFI surface.
///
/// Mirrors [`ServerSyncError`] plus client-side input/derivation failures.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiSyncError {
    /// The host transport failed (network, DNS, TLS, …). Retryable.
    #[error("transport error: {message}")]
    Transport { message: String },
    /// The server returned a non-2xx status.
    #[error("server returned status {status}: {message}")]
    Http { status: u16, message: String },
    /// A response body could not be decoded.
    #[error("failed to decode response: {message}")]
    Decode { message: String },
    /// The server's SRP proof (M2) did not verify.
    #[error("server authentication proof mismatch")]
    ProofMismatch,
    /// An authenticated operation was attempted without a session token.
    #[error("not authenticated")]
    NotAuthenticated,
    /// SRP handshake failure.
    #[error("srp error: {message}")]
    Srp { message: String },
    /// Invalid client input (bad UUID, malformed Secret Key, key derivation …).
    #[error("invalid input: {message}")]
    InvalidInput { message: String },
}

impl From<ServerSyncError> for FfiSyncError {
    fn from(e: ServerSyncError) -> Self {
        match e {
            ServerSyncError::Transport(message) => Self::Transport { message },
            ServerSyncError::Http { status, message } => Self::Http { status, message },
            ServerSyncError::Decode(message) => Self::Decode { message },
            ServerSyncError::ProofMismatch => Self::ProofMismatch,
            ServerSyncError::NotAuthenticated => Self::NotAuthenticated,
            ServerSyncError::Srp(e) => Self::Srp {
                message: e.to_string(),
            },
            ServerSyncError::Hex(e) => Self::Decode {
                message: e.to_string(),
            },
        }
    }
}

/// Convert an [`FfiSyncError`] returned by the foreign sender back into a core
/// [`ServerSyncError`] so the core client classifies it correctly. In practice
/// a well-behaved host only returns [`FfiSyncError::Transport`] (a non-2xx HTTP
/// response is *not* an error — it is returned as an [`FfiRawResponse`]).
fn ffi_error_to_core(e: FfiSyncError) -> ServerSyncError {
    match e {
        FfiSyncError::Transport { message }
        | FfiSyncError::Srp { message }
        | FfiSyncError::InvalidInput { message } => ServerSyncError::Transport(message),
        FfiSyncError::Http { status, message } => ServerSyncError::Http { status, message },
        FfiSyncError::Decode { message } => ServerSyncError::Decode(message),
        FfiSyncError::ProofMismatch => ServerSyncError::ProofMismatch,
        FfiSyncError::NotAuthenticated => ServerSyncError::NotAuthenticated,
    }
}

// ── Raw transport types ──────────────────────────────────────────────────────

/// HTTP method for a raw request.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum FfiHttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl From<HttpMethod> for FfiHttpMethod {
    fn from(m: HttpMethod) -> Self {
        match m {
            HttpMethod::Get => Self::Get,
            HttpMethod::Post => Self::Post,
            HttpMethod::Put => Self::Put,
            HttpMethod::Delete => Self::Delete,
        }
    }
}

/// A single header or query key/value pair (`UniFFI` has no tuple type).
#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiKeyValue {
    pub name: String,
    pub value: String,
}

/// A transport-agnostic HTTP request the host must execute.
///
/// `path` is absolute (e.g. `/api/v1/auth/register`); `query` holds
/// not-yet-encoded key/value pairs the host should append as a query string.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiRawRequest {
    pub method: FfiHttpMethod,
    pub path: String,
    pub query: Vec<FfiKeyValue>,
    pub headers: Vec<FfiKeyValue>,
    pub body: Vec<u8>,
}

impl From<RawRequest> for FfiRawRequest {
    fn from(r: RawRequest) -> Self {
        let map = |pairs: Vec<(String, String)>| {
            pairs
                .into_iter()
                .map(|(name, value)| FfiKeyValue { name, value })
                .collect()
        };
        Self {
            method: r.method.into(),
            path: r.path,
            query: map(r.query),
            headers: map(r.headers),
            body: r.body,
        }
    }
}

/// The raw HTTP response returned by the host transport.
///
/// A non-2xx `status` is **not** an error — return it here and let the core
/// client classify it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiRawResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

// ── Foreign sender (the I/O seam) ────────────────────────────────────────────

/// Host-implemented async HTTP sender.
///
/// Swift implements this over `URLSession`; web implements the equivalent seam
/// over `fetch` in the separate `ldgr-wasm` crate. Implementors perform the
/// network request and map transport-level failures into
/// [`FfiSyncError::Transport`].
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait FfiHttpSender: Send + Sync {
    /// Execute `request` and return the raw response.
    async fn send(&self, request: FfiRawRequest) -> Result<FfiRawResponse, FfiSyncError>;
}

/// Adapter that lets a foreign [`FfiHttpSender`] satisfy the core
/// [`RawHttpSender`] contract.
struct ForeignSender {
    inner: Arc<dyn FfiHttpSender>,
}

impl RawHttpSender for ForeignSender {
    async fn send(&self, request: RawRequest) -> Result<RawResponse, ServerSyncError> {
        let resp = self
            .inner
            .send(request.into())
            .await
            .map_err(ffi_error_to_core)?;
        Ok(RawResponse {
            status: resp.status,
            body: resp.body,
        })
    }
}

// ── Auth / blob result records ───────────────────────────────────────────────

/// Argon2id parameters for deriving `MK_auth` during 2SKD auth.
#[derive(Debug, Clone, Copy, uniffi::Record)]
pub struct FfiArgon2Params {
    pub memory_cost_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl From<FfiArgon2Params> for Argon2Params {
    fn from(p: FfiArgon2Params) -> Self {
        Self {
            memory_cost_kib: p.memory_cost_kib,
            iterations: p.iterations,
            parallelism: p.parallelism,
        }
    }
}

/// Result of an opaque blob upload.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiPutBlobResult {
    pub path: String,
    pub size: u64,
    pub content_hash: String,
}

/// Parsed metadata about a remote event batch.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiRemoteBatchMeta {
    pub batch_id: String,
    pub device_id: String,
    pub path: String,
    pub size: u64,
    pub content_hash: Option<String>,
    pub modified_at: Option<String>,
}

/// Parsed metadata about a remote snapshot.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiRemoteSnapshotMeta {
    pub snapshot_id: String,
    pub path: String,
    pub size: u64,
    pub content_hash: Option<String>,
    pub modified_at: Option<String>,
}

/// A registered device (its `encrypted_info` is hex-encoded ciphertext, opaque
/// to the server).
#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiDevice {
    pub id: String,
    pub updated_at: String,
    pub encrypted_info: String,
}

// ── Client ───────────────────────────────────────────────────────────────────

/// FFI handle wrapping a [`ServerSyncClient`] driven by a foreign sender.
///
/// Methods are async and serialized behind a [`futures::lock::Mutex`] because
/// the SRP login handshake needs `&mut` access to the underlying client across
/// an `.await`.
#[derive(uniffi::Object)]
pub struct LdgrSyncClient {
    inner: Mutex<ServerSyncClient<ForeignSender>>,
}

#[uniffi::export]
impl LdgrSyncClient {
    /// Create a client with no active session.
    #[uniffi::constructor]
    pub fn new(sender: Arc<dyn FfiHttpSender>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ServerSyncClient::new(ForeignSender { inner: sender })),
        })
    }

    /// Create a client that resumes a previously persisted session token.
    #[uniffi::constructor]
    pub fn with_token(sender: Arc<dyn FfiHttpSender>, token: String) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ServerSyncClient::with_token(
                ForeignSender { inner: sender },
                token,
            )),
        })
    }

    /// The current bearer token, if authenticated. Persist this on the host to
    /// later reconstruct the client via [`with_token`](Self::with_token).
    pub async fn token(&self) -> Option<String> {
        self.inner.lock().await.token().map(str::to_string)
    }

    /// Whether the client holds a session token.
    pub async fn is_authenticated(&self) -> bool {
        self.inner.lock().await.is_authenticated()
    }

    // ── Auth: single-secret ──────────────────────────────────────────────────

    /// Register a new account (single-secret SRP). Returns the user id.
    pub async fn register(
        &self,
        username: String,
        password: Vec<u8>,
    ) -> Result<String, FfiSyncError> {
        let client = self.inner.lock().await;
        Ok(client.register(&username, &password).await?.user_id)
    }

    /// Perform a single-secret SRP-6a login and store the session token.
    pub async fn login(&self, username: String, password: Vec<u8>) -> Result<(), FfiSyncError> {
        let mut client = self.inner.lock().await;
        client.login(&username, &password).await?;
        Ok(())
    }

    // ── Auth: two-secret (2SKD, ADR-008) ─────────────────────────────────────

    /// Register a new account using two-secret (2SKD) derivation.
    ///
    /// Derives `MK_auth` from `password` + `argon2_salt`/`argon2_params` and
    /// parses `secret_key` (canonical text form) — both stay in Rust.
    pub async fn register_2skd(
        &self,
        username: String,
        account_id: String,
        password: Vec<u8>,
        secret_key: String,
        argon2_salt: Vec<u8>,
        argon2_params: FfiArgon2Params,
    ) -> Result<String, FfiSyncError> {
        let (aid, mk_auth, sk) = derive_2skd(
            &account_id,
            &password,
            &secret_key,
            &argon2_salt,
            argon2_params,
        )?;
        let client = self.inner.lock().await;
        Ok(client
            .register_2skd(&username, &aid, &mk_auth, &sk)
            .await?
            .user_id)
    }

    /// Perform a two-secret (2SKD) SRP-6a login and store the session token.
    pub async fn login_2skd(
        &self,
        username: String,
        account_id: String,
        password: Vec<u8>,
        secret_key: String,
        argon2_salt: Vec<u8>,
        argon2_params: FfiArgon2Params,
    ) -> Result<(), FfiSyncError> {
        let (aid, mk_auth, sk) = derive_2skd(
            &account_id,
            &password,
            &secret_key,
            &argon2_salt,
            argon2_params,
        )?;
        let mut client = self.inner.lock().await;
        client.login_2skd(&username, &aid, &mk_auth, &sk).await?;
        Ok(())
    }

    // ── Vaults ───────────────────────────────────────────────────────────────

    /// Create a vault. Returns the server-assigned vault id.
    pub async fn create_vault(&self, vault_id: String) -> Result<String, FfiSyncError> {
        let client = self.inner.lock().await;
        Ok(client.create_vault(&vault_id).await?.id)
    }

    // ── Batches (opaque ciphertext) ──────────────────────────────────────────

    /// Upload an encrypted event batch (put-if-absent).
    pub async fn put_batch(
        &self,
        vault_id: String,
        device_id: String,
        batch_id: String,
        ciphertext: Vec<u8>,
    ) -> Result<FfiPutBlobResult, FfiSyncError> {
        let client = self.inner.lock().await;
        let resp = client
            .put_batch(&vault_id, &device_id, &batch_id, &ciphertext)
            .await?;
        Ok(put_result(resp))
    }

    /// Download an encrypted event batch.
    pub async fn get_batch(
        &self,
        vault_id: String,
        device_id: String,
        batch_id: String,
    ) -> Result<Vec<u8>, FfiSyncError> {
        let client = self.inner.lock().await;
        Ok(client.get_batch(&vault_id, &device_id, &batch_id).await?)
    }

    /// List remote event batches as parsed metadata.
    pub async fn list_remote_batches(
        &self,
        vault_id: String,
        since: Option<String>,
        device_id: Option<String>,
        limit: Option<u32>,
    ) -> Result<Vec<FfiRemoteBatchMeta>, FfiSyncError> {
        let query = ListBatchesQuery {
            since,
            device_id,
            limit,
        };
        let client = self.inner.lock().await;
        let metas = client.list_remote_batches(&vault_id, &query).await?;
        Ok(metas
            .into_iter()
            .map(|m| FfiRemoteBatchMeta {
                batch_id: m.batch_id,
                device_id: m.device_id,
                path: m.path,
                size: m.size,
                content_hash: m.content_hash,
                modified_at: m.modified_at,
            })
            .collect())
    }

    // ── Snapshots (opaque ciphertext) ────────────────────────────────────────

    /// Upload an encrypted snapshot (put-if-absent).
    pub async fn put_snapshot(
        &self,
        vault_id: String,
        snapshot_id: String,
        ciphertext: Vec<u8>,
    ) -> Result<FfiPutBlobResult, FfiSyncError> {
        let client = self.inner.lock().await;
        let resp = client
            .put_snapshot(&vault_id, &snapshot_id, &ciphertext)
            .await?;
        Ok(put_result(resp))
    }

    /// Download an encrypted snapshot.
    pub async fn get_snapshot(
        &self,
        vault_id: String,
        snapshot_id: String,
    ) -> Result<Vec<u8>, FfiSyncError> {
        let client = self.inner.lock().await;
        Ok(client.get_snapshot(&vault_id, &snapshot_id).await?)
    }

    /// List remote snapshots as parsed metadata.
    pub async fn list_remote_snapshots(
        &self,
        vault_id: String,
        since: Option<String>,
        limit: Option<u32>,
    ) -> Result<Vec<FfiRemoteSnapshotMeta>, FfiSyncError> {
        let query = ListSnapshotsQuery { since, limit };
        let client = self.inner.lock().await;
        let metas = client.list_remote_snapshots(&vault_id, &query).await?;
        Ok(metas
            .into_iter()
            .map(|m| FfiRemoteSnapshotMeta {
                snapshot_id: m.snapshot_id,
                path: m.path,
                size: m.size,
                content_hash: m.content_hash,
                modified_at: m.modified_at,
            })
            .collect())
    }

    // ── Devices ──────────────────────────────────────────────────────────────

    /// Register or update a device with opaque encrypted info.
    pub async fn put_device(
        &self,
        vault_id: String,
        device_id: String,
        encrypted_info: Vec<u8>,
    ) -> Result<(), FfiSyncError> {
        let client = self.inner.lock().await;
        client
            .put_device(&vault_id, &device_id, &encrypted_info)
            .await?;
        Ok(())
    }

    /// List registered devices for a vault.
    pub async fn list_devices(&self, vault_id: String) -> Result<Vec<FfiDevice>, FfiSyncError> {
        let client = self.inner.lock().await;
        let devices = client.list_devices(&vault_id).await?;
        Ok(devices
            .into_iter()
            .map(|d| FfiDevice {
                id: d.id,
                updated_at: d.updated_at,
                encrypted_info: d.encrypted_info,
            })
            .collect())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn put_result(resp: ldgr_core::sync::server::PutBlobResponse) -> FfiPutBlobResult {
    FfiPutBlobResult {
        path: resp.path,
        #[allow(clippy::cast_sign_loss)]
        size: resp.size.max(0) as u64,
        content_hash: resp.content_hash,
    }
}

/// Derive the 2SKD login/registration inputs from human-provided secrets,
/// keeping all key material inside Rust.
fn derive_2skd(
    account_id: &str,
    password: &[u8],
    secret_key: &str,
    argon2_salt: &[u8],
    argon2_params: FfiArgon2Params,
) -> Result<(Uuid, ldgr_core::crypto::AuthKey, SecretKey), FfiSyncError> {
    let aid = Uuid::parse_str(account_id).map_err(|e| FfiSyncError::InvalidInput {
        message: format!("invalid account id: {e}"),
    })?;
    let params: Argon2Params = argon2_params.into();
    let master_key = derive_master_key(password, argon2_salt, &params).map_err(|e| {
        FfiSyncError::InvalidInput {
            message: format!("key derivation failed: {e}"),
        }
    })?;
    let mk_auth = derive_auth_key(&master_key).map_err(|e| FfiSyncError::InvalidInput {
        message: format!("key derivation failed: {e}"),
    })?;
    let sk = SecretKey::parse(secret_key).map_err(|e| FfiSyncError::InvalidInput {
        message: format!("invalid secret key: {e}"),
    })?;
    Ok((aid, mk_auth, sk))
}
