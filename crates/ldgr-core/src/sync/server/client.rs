//! Transport-agnostic server sync client.
//!
//! [`ServerSyncClient`] orchestrates the SRP-6a auth handshake and the
//! encrypted batch/snapshot/device/relay endpoints of `ldgr-server`. It does
//! **no I/O itself**: every request is handed to an injected [`RawHttpSender`]
//! that the platform implements (CLI/reqwest, iOS/URLSession, web/fetch). This
//! keeps `ldgr-core` free of networking while sharing the protocol logic across
//! all clients.
//!
//! The client builds on the canonical blob layout defined in
//! [`crate::sync::transport`] and produces/consumes the ciphertext blobs created
//! by [`crate::sync::events`] and [`crate::sync::snapshot`].

use zeroize::Zeroizing;

use super::protocol::{
    CreateOfferRequest, CreateOfferResponse, CreateVaultRequest, DeviceResponse, ErrorResponse,
    GetResponseResponse, HexError, ListBatchesQuery, ListBlobsResponse, ListSnapshotsQuery,
    LoginInitRequest, LoginInitResponse, LoginVerifyRequest, LoginVerifyResponse, OfferResponse,
    Pong, PostResponseRequest, PutBlobResponse, RegisterRequest, RegisterResponse, ServerInfo,
    VaultResponse, hex_decode, hex_encode,
};
use super::srp::{ClientLogin, SrpError};
use crate::crypto::{AuthKey, SecretKey};
use crate::sync::transport::{
    RemoteBatchMeta, RemoteSnapshotMeta, parse_batch_path, parse_snapshot_path,
};
use uuid::Uuid;

// ── Paginated list results ──────────────────────────────────────────────────────

/// One page of batch metadata plus the keyset cursor to fetch the next page.
#[derive(Debug, Clone)]
pub struct RemoteBatchPage {
    /// Parsed batch metadata for this page.
    pub metas: Vec<RemoteBatchMeta>,
    /// Whether the server has more pages beyond this one.
    pub has_more: bool,
    /// Opaque cursor to pass as the next query's `since` when `has_more` is
    /// true. `None` on the final page.
    pub next_cursor: Option<String>,
}

/// One page of snapshot metadata plus the keyset cursor to fetch the next page.
#[derive(Debug, Clone)]
pub struct RemoteSnapshotPage {
    /// Parsed snapshot metadata for this page.
    pub metas: Vec<RemoteSnapshotMeta>,
    /// Whether the server has more pages beyond this one.
    pub has_more: bool,
    /// Opaque cursor to pass as the next query's `since` when `has_more` is
    /// true. `None` on the final page.
    pub next_cursor: Option<String>,
}

// ── Raw transport contract ──────────────────────────────────────────────────────

/// HTTP method for a raw request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl HttpMethod {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

/// A transport-agnostic HTTP request the platform must execute.
///
/// `path` is the absolute path (e.g. `/api/v1/auth/register`); `query` holds
/// not-yet-encoded key/value pairs the platform should append as a query string.
#[derive(Debug, Clone)]
pub struct RawRequest {
    pub method: HttpMethod,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The raw HTTP response returned by the platform transport.
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl RawResponse {
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Platform-implemented async HTTP sender.
///
/// Implementors perform the actual network request and map transport-level
/// failures into [`ServerSyncError::Transport`]. A non-2xx HTTP response is
/// **not** an error here — return it as a [`RawResponse`] and let the client
/// classify it.
#[allow(async_fn_in_trait)] // futures need not be Send (WASM single-threaded).
pub trait RawHttpSender {
    async fn send(&self, request: RawRequest) -> Result<RawResponse, ServerSyncError>;
}

// ── Errors ─────────────────────────────────────────────────────────────────────

/// Errors raised by [`ServerSyncClient`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServerSyncError {
    /// The injected transport failed (network, DNS, TLS, …).
    #[error("transport error: {0}")]
    Transport(String),
    /// The server returned a non-2xx status.
    #[error("server returned status {status}: {message}")]
    Http { status: u16, message: String },
    /// A response body could not be decoded.
    #[error("failed to decode response: {0}")]
    Decode(String),
    /// The server's SRP proof (M2) did not verify.
    #[error("server authentication proof mismatch")]
    ProofMismatch,
    /// An authenticated operation was attempted without a session token.
    #[error("not authenticated")]
    NotAuthenticated,
    /// SRP handshake failure.
    #[error(transparent)]
    Srp(#[from] SrpError),
    /// A hex-encoded field in a response was malformed.
    #[error("invalid hex: {0}")]
    Hex(#[from] HexError),
}

// ── Client ─────────────────────────────────────────────────────────────────────

const API_PREFIX: &str = "/api/v1";

/// Orchestrates the `ldgr-server` protocol over an injected transport.
pub struct ServerSyncClient<T> {
    sender: T,
    token: Option<String>,
    session_key: Option<Zeroizing<Vec<u8>>>,
}

impl<T: RawHttpSender> ServerSyncClient<T> {
    /// Create a client with no active session.
    #[must_use]
    pub fn new(sender: T) -> Self {
        Self {
            sender,
            token: None,
            session_key: None,
        }
    }

    /// Create a client that resumes a previously obtained session token.
    #[must_use]
    pub fn with_token(sender: T, token: String) -> Self {
        Self {
            sender,
            token: Some(token),
            session_key: None,
        }
    }

    /// The current bearer token, if authenticated.
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Whether the client holds a session token.
    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    /// The SRP session key `K` derived during [`login`](Self::login), if any.
    #[must_use]
    pub fn session_key(&self) -> Option<&[u8]> {
        self.session_key.as_deref().map(|k| &k[..])
    }

    /// Drop the current session.
    pub fn logout_local(&mut self) {
        self.token = None;
        self.session_key = None;
    }

    // ── Auth ────────────────────────────────────────────────────────────────

    /// Register a new account. Returns the server-assigned user id.
    ///
    /// Derives the SRP salt + verifier locally; the password never leaves the
    /// device.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport fails or the server rejects the
    /// request.
    pub async fn register(
        &self,
        username: &str,
        password: &[u8],
    ) -> Result<RegisterResponse, ServerSyncError> {
        let reg = super::srp::register(username, password);
        let body = RegisterRequest {
            username: username.to_string(),
            salt: hex_encode(&reg.salt),
            verifier: hex_encode(&reg.verifier),
            auth_scheme: None,
            account_id: None,
        };
        self.post_json(&format!("{API_PREFIX}/auth/register"), &body, false)
            .await
    }

    /// Register a new account using **two-secret (2SKD)** derivation (ADR-008).
    ///
    /// The SRP verifier is derived from both the master auth key (`mk_auth`,
    /// derived from the password) and the account [`SecretKey`]; the request
    /// advertises `auth_scheme = "srp-2skd-v1"`. Neither secret leaves the
    /// device — only `(salt, verifier)` is sent.
    ///
    /// The Secret Key is **server-auth-only** (ADR-008, Decision 3): it
    /// participates in account authentication and is never used to decrypt the
    /// vault.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport fails or the server rejects the
    /// request.
    pub async fn register_2skd(
        &self,
        username: &str,
        account_id: &Uuid,
        mk_auth: &AuthKey,
        secret_key: &SecretKey,
    ) -> Result<RegisterResponse, ServerSyncError> {
        let reg = super::srp::register_2skd(account_id, mk_auth, secret_key);
        let body = RegisterRequest {
            username: username.to_string(),
            salt: hex_encode(&reg.salt),
            verifier: hex_encode(&reg.verifier),
            auth_scheme: Some("srp-2skd-v1".to_string()),
            account_id: Some(account_id.to_string()),
        };
        self.post_json(&format!("{API_PREFIX}/auth/register"), &body, false)
            .await
    }

    /// Perform the full SRP-6a login handshake and store the session token.
    ///
    /// On success the client becomes authenticated ([`is_authenticated`] is
    /// true) and the SRP session key is retained.
    ///
    /// # Errors
    ///
    /// Returns [`ServerSyncError::ProofMismatch`] if the server's proof fails to
    /// verify, an SRP error for a malformed handshake, or a transport/HTTP
    /// error.
    ///
    /// [`is_authenticated`]: Self::is_authenticated
    pub async fn login(&mut self, username: &str, password: &[u8]) -> Result<(), ServerSyncError> {
        let (login, a_pub) = ClientLogin::start(username, password);
        self.run_login(username, login, a_pub).await
    }

    /// Perform a **two-secret (2SKD)** SRP-6a login and store the session token
    /// (ADR-008).
    ///
    /// `mk_auth` is the master auth key derived from the password; `secret_key`
    /// is the account [`SecretKey`]. Both are required to reproduce the
    /// verifier — neither alone authenticates. The account id is learned from
    /// the server's `login/init` response, so callers need not supply it.
    ///
    /// The Secret Key is **server-auth-only** (ADR-008, Decision 3): it is used
    /// solely to prove account ownership to the server and is never used for
    /// vault decryption.
    ///
    /// # Errors
    ///
    /// Returns [`ServerSyncError::ProofMismatch`] if the server's proof fails to
    /// verify, [`ServerSyncError::Srp`] with [`SrpError::MissingAccountId`] if
    /// the server did not return an account id (e.g. a legacy single-secret
    /// account), or a transport/HTTP error.
    ///
    /// [`SrpError::MissingAccountId`]: super::srp::SrpError::MissingAccountId
    pub async fn login_2skd(
        &mut self,
        username: &str,
        mk_auth: &AuthKey,
        secret_key: &SecretKey,
    ) -> Result<(), ServerSyncError> {
        let (login, a_pub) = ClientLogin::start_2skd(username, mk_auth.clone(), secret_key.clone());
        self.run_login(username, login, a_pub).await
    }

    /// Drive the SRP-6a init → verify exchange common to every auth scheme.
    ///
    /// `login` is the started [`ClientLogin`] state and `a_pub` its public
    /// value `A`. On success the client becomes authenticated and the SRP
    /// session key is retained.
    async fn run_login(
        &mut self,
        username: &str,
        mut login: ClientLogin,
        a_pub: Vec<u8>,
    ) -> Result<(), ServerSyncError> {
        // Step 1: init.
        let init_req = LoginInitRequest {
            username: username.to_string(),
            client_public: hex_encode(&a_pub),
        };
        let init: LoginInitResponse = self
            .post_json(&format!("{API_PREFIX}/auth/login/init"), &init_req, false)
            .await?;

        // For two-secret accounts the server returns the stored account id,
        // which the client needs to derive `x` (ADR-008). Inject it before
        // finishing; a no-op for single-secret logins.
        if let Some(account_id) = &init.account_id {
            let account_id = Uuid::parse_str(account_id)
                .map_err(|e| ServerSyncError::Decode(format!("invalid account_id: {e}")))?;
            login.set_account_id(account_id);
        }

        let salt = hex_decode(&init.salt)?;
        let server_public = hex_decode(&init.server_public)?;
        let session = login.finish(&salt, &server_public)?;

        // Step 2: verify.
        let verify_req = LoginVerifyRequest {
            handshake_id: init.handshake_id,
            client_proof: hex_encode(session.proof()),
        };
        let verify: LoginVerifyResponse = self
            .post_json(
                &format!("{API_PREFIX}/auth/login/verify"),
                &verify_req,
                false,
            )
            .await?;

        let server_proof = hex_decode(&verify.server_proof)?;
        if !session.verify_server_proof(&server_proof) {
            return Err(ServerSyncError::ProofMismatch);
        }

        self.token = Some(verify.token);
        self.session_key = Some(Zeroizing::new(session.session_key().to_vec()));
        Ok(())
    }

    // ── Discovery (unauthenticated) ───────────────────────────────────────────

    /// Fetch the server's discovery document (`GET /server/info`).
    ///
    /// Use before sign-in to validate a URL points at an ldgr server, learn the
    /// wire-protocol version, and read the registration policy / auth
    /// capabilities so the client can render the right onboarding flow (ADR-008,
    /// #177). Unauthenticated.
    ///
    /// # Errors
    ///
    /// Returns a transport/HTTP error, or [`ServerSyncError::Decode`] if the
    /// response is not a valid discovery document.
    pub async fn server_info(&self) -> Result<ServerInfo, ServerSyncError> {
        self.get_json(&format!("{API_PREFIX}/server/info"), &[], false)
            .await
    }

    /// Cheap liveness probe (`GET /server/ping`) for URL validation.
    ///
    /// # Errors
    ///
    /// Returns a transport/HTTP error, or [`ServerSyncError::Decode`] if the
    /// response body is not a valid pong.
    pub async fn ping(&self) -> Result<Pong, ServerSyncError> {
        self.get_json(&format!("{API_PREFIX}/server/ping"), &[], false)
            .await
    }

    // ── Vaults ──────────────────────────────────────────────────────────────

    /// Create a vault.
    pub async fn create_vault(&self, vault_id: &str) -> Result<VaultResponse, ServerSyncError> {
        let body = CreateVaultRequest {
            vault_id: vault_id.to_string(),
        };
        self.post_json(&format!("{API_PREFIX}/vaults"), &body, true)
            .await
    }

    /// List vaults owned by the authenticated user.
    pub async fn list_vaults(&self) -> Result<Vec<VaultResponse>, ServerSyncError> {
        self.get_json(&format!("{API_PREFIX}/vaults"), &[], true)
            .await
    }

    // ── Batches ─────────────────────────────────────────────────────────────

    /// Upload an encrypted event batch (put-if-absent).
    pub async fn put_batch(
        &self,
        vault_id: &str,
        device_id: &str,
        batch_id: &str,
        ciphertext: &[u8],
    ) -> Result<PutBlobResponse, ServerSyncError> {
        let path = format!("{API_PREFIX}/vaults/{vault_id}/batches/{device_id}/{batch_id}");
        self.put_bytes(&path, ciphertext).await
    }

    /// Download an encrypted event batch.
    pub async fn get_batch(
        &self,
        vault_id: &str,
        device_id: &str,
        batch_id: &str,
    ) -> Result<Vec<u8>, ServerSyncError> {
        let path = format!("{API_PREFIX}/vaults/{vault_id}/batches/{device_id}/{batch_id}");
        self.get_bytes(&path).await
    }

    /// List batch blobs.
    pub async fn list_batches(
        &self,
        vault_id: &str,
        query: &ListBatchesQuery,
    ) -> Result<ListBlobsResponse, ServerSyncError> {
        let mut params = Vec::new();
        if let Some(since) = &query.since {
            params.push(("since".to_string(), since.clone()));
        }
        if let Some(device_id) = &query.device_id {
            params.push(("device_id".to_string(), device_id.clone()));
        }
        if let Some(limit) = query.limit {
            params.push(("limit".to_string(), limit.to_string()));
        }
        self.get_json(
            &format!("{API_PREFIX}/vaults/{vault_id}/batches"),
            &params,
            true,
        )
        .await
    }

    /// List batch blobs as parsed [`RemoteBatchMeta`], skipping any paths that
    /// don't match the canonical batch layout.
    ///
    /// Returns a single page. Callers that need every batch in a large vault
    /// should use [`Self::list_remote_batches_page`] and follow the returned
    /// cursor until `has_more` is false.
    pub async fn list_remote_batches(
        &self,
        vault_id: &str,
        query: &ListBatchesQuery,
    ) -> Result<Vec<RemoteBatchMeta>, ServerSyncError> {
        Ok(self.list_remote_batches_page(vault_id, query).await?.metas)
    }

    /// List a single page of batch blobs, exposing the pagination cursor.
    ///
    /// When `has_more` is true, pass `next_cursor` as the `since` field of the
    /// next query to continue from where this page ended. The cursor is an
    /// opaque keyset token over `(created_at, path)`, so pages never overlap or
    /// skip a blob even when many share a timestamp.
    pub async fn list_remote_batches_page(
        &self,
        vault_id: &str,
        query: &ListBatchesQuery,
    ) -> Result<RemoteBatchPage, ServerSyncError> {
        let resp = self.list_batches(vault_id, query).await?;
        let has_more = resp.has_more;
        let next_cursor = resp.cursor;
        let metas = resp
            .entries
            .into_iter()
            .filter_map(|e| {
                let r = parse_batch_path(&e.path)?;
                #[allow(clippy::cast_sign_loss)]
                Some(RemoteBatchMeta {
                    batch_id: r.batch_id,
                    device_id: r.device_id,
                    path: e.path,
                    size: e.size.max(0) as u64,
                    content_hash: Some(e.content_hash),
                    modified_at: Some(e.created_at),
                })
            })
            .collect();
        Ok(RemoteBatchPage {
            metas,
            has_more,
            next_cursor,
        })
    }

    // ── Snapshots ───────────────────────────────────────────────────────────

    /// Upload an encrypted snapshot (put-if-absent).
    pub async fn put_snapshot(
        &self,
        vault_id: &str,
        snapshot_id: &str,
        ciphertext: &[u8],
    ) -> Result<PutBlobResponse, ServerSyncError> {
        let path = format!("{API_PREFIX}/vaults/{vault_id}/snapshots/{snapshot_id}");
        self.put_bytes(&path, ciphertext).await
    }

    /// Download an encrypted snapshot.
    pub async fn get_snapshot(
        &self,
        vault_id: &str,
        snapshot_id: &str,
    ) -> Result<Vec<u8>, ServerSyncError> {
        let path = format!("{API_PREFIX}/vaults/{vault_id}/snapshots/{snapshot_id}");
        self.get_bytes(&path).await
    }

    /// List snapshot blobs.
    pub async fn list_snapshots(
        &self,
        vault_id: &str,
        query: &ListSnapshotsQuery,
    ) -> Result<ListBlobsResponse, ServerSyncError> {
        let mut params = Vec::new();
        if let Some(since) = &query.since {
            params.push(("since".to_string(), since.clone()));
        }
        if let Some(limit) = query.limit {
            params.push(("limit".to_string(), limit.to_string()));
        }
        self.get_json(
            &format!("{API_PREFIX}/vaults/{vault_id}/snapshots"),
            &params,
            true,
        )
        .await
    }

    /// List snapshot blobs as parsed [`RemoteSnapshotMeta`].
    ///
    /// Returns a single page. Use [`Self::list_remote_snapshots_page`] to follow
    /// the cursor across pages.
    pub async fn list_remote_snapshots(
        &self,
        vault_id: &str,
        query: &ListSnapshotsQuery,
    ) -> Result<Vec<RemoteSnapshotMeta>, ServerSyncError> {
        Ok(self
            .list_remote_snapshots_page(vault_id, query)
            .await?
            .metas)
    }

    /// List a single page of snapshot blobs, exposing the pagination cursor.
    ///
    /// See [`Self::list_remote_batches_page`] for the cursor-following contract.
    pub async fn list_remote_snapshots_page(
        &self,
        vault_id: &str,
        query: &ListSnapshotsQuery,
    ) -> Result<RemoteSnapshotPage, ServerSyncError> {
        let resp = self.list_snapshots(vault_id, query).await?;
        let has_more = resp.has_more;
        let next_cursor = resp.cursor;
        let metas = resp
            .entries
            .into_iter()
            .filter_map(|e| {
                let id = parse_snapshot_path(&e.path)?;
                #[allow(clippy::cast_sign_loss)]
                Some(RemoteSnapshotMeta {
                    snapshot_id: id,
                    path: e.path,
                    size: e.size.max(0) as u64,
                    content_hash: Some(e.content_hash),
                    modified_at: Some(e.created_at),
                })
            })
            .collect();
        Ok(RemoteSnapshotPage {
            metas,
            has_more,
            next_cursor,
        })
    }

    // ── Devices ─────────────────────────────────────────────────────────────

    /// List registered devices for a vault.
    pub async fn list_devices(
        &self,
        vault_id: &str,
    ) -> Result<Vec<DeviceResponse>, ServerSyncError> {
        self.get_json(
            &format!("{API_PREFIX}/vaults/{vault_id}/devices"),
            &[],
            true,
        )
        .await
    }

    /// Register or update a device with opaque encrypted info.
    pub async fn put_device(
        &self,
        vault_id: &str,
        device_id: &str,
        encrypted_info: &[u8],
    ) -> Result<(), ServerSyncError> {
        let path = format!("{API_PREFIX}/vaults/{vault_id}/devices/{device_id}");
        let _ = self
            .send_expecting(HttpMethod::Put, &path, &[], encrypted_info.to_vec(), true)
            .await?;
        Ok(())
    }

    /// Remove a device.
    pub async fn delete_device(
        &self,
        vault_id: &str,
        device_id: &str,
    ) -> Result<(), ServerSyncError> {
        let path = format!("{API_PREFIX}/vaults/{vault_id}/devices/{device_id}");
        let _ = self
            .send_expecting(HttpMethod::Delete, &path, &[], Vec::new(), true)
            .await?;
        Ok(())
    }

    // ── Relay ───────────────────────────────────────────────────────────────

    /// Create a key-exchange relay offer.
    pub async fn create_offer(
        &self,
        offer_data: &[u8],
    ) -> Result<CreateOfferResponse, ServerSyncError> {
        let body = CreateOfferRequest {
            offer_data: hex_encode(offer_data),
        };
        self.post_json(&format!("{API_PREFIX}/relay/offer"), &body, true)
            .await
    }

    /// Fetch a relay offer.
    pub async fn get_offer(&self, offer_id: &str) -> Result<OfferResponse, ServerSyncError> {
        self.get_json(&format!("{API_PREFIX}/relay/{offer_id}"), &[], true)
            .await
    }

    /// Post a response to a relay offer.
    pub async fn post_offer_response(
        &self,
        offer_id: &str,
        response_data: &[u8],
    ) -> Result<(), ServerSyncError> {
        let body = PostResponseRequest {
            response_data: hex_encode(response_data),
        };
        let payload =
            serde_json::to_vec(&body).map_err(|e| ServerSyncError::Decode(e.to_string()))?;
        let _ = self
            .send_expecting(
                HttpMethod::Post,
                &format!("{API_PREFIX}/relay/{offer_id}/respond"),
                &[("content-type".to_string(), "application/json".to_string())],
                payload,
                true,
            )
            .await?;
        Ok(())
    }

    /// Fetch the decrypted-on-server-opaque response payload for an offer.
    pub async fn get_offer_response(&self, offer_id: &str) -> Result<Vec<u8>, ServerSyncError> {
        let resp: GetResponseResponse = self
            .get_json(
                &format!("{API_PREFIX}/relay/{offer_id}/response"),
                &[],
                true,
            )
            .await?;
        Ok(hex_decode(&resp.response_data)?)
    }

    // ── Internal request helpers ──────────────────────────────────────────────

    fn auth_headers(&self, auth: bool) -> Result<Vec<(String, String)>, ServerSyncError> {
        if !auth {
            return Ok(Vec::new());
        }
        let token = self
            .token
            .as_ref()
            .ok_or(ServerSyncError::NotAuthenticated)?;
        Ok(vec![(
            "authorization".to_string(),
            format!("Bearer {token}"),
        )])
    }

    async fn post_json<Req, Res>(
        &self,
        path: &str,
        body: &Req,
        auth: bool,
    ) -> Result<Res, ServerSyncError>
    where
        Req: serde::Serialize,
        Res: serde::de::DeserializeOwned,
    {
        let mut headers = self.auth_headers(auth)?;
        headers.push(("content-type".to_string(), "application/json".to_string()));
        let payload =
            serde_json::to_vec(body).map_err(|e| ServerSyncError::Decode(e.to_string()))?;
        let resp = self
            .send_expecting(HttpMethod::Post, path, &headers, payload, auth)
            .await?;
        decode_json(&resp.body)
    }

    async fn get_json<Res>(
        &self,
        path: &str,
        query: &[(String, String)],
        auth: bool,
    ) -> Result<Res, ServerSyncError>
    where
        Res: serde::de::DeserializeOwned,
    {
        let headers = self.auth_headers(auth)?;
        let request = RawRequest {
            method: HttpMethod::Get,
            path: path.to_string(),
            query: query.to_vec(),
            headers,
            body: Vec::new(),
        };
        let resp = self.dispatch(request).await?;
        decode_json(&resp.body)
    }

    async fn get_bytes(&self, path: &str) -> Result<Vec<u8>, ServerSyncError> {
        let headers = self.auth_headers(true)?;
        let request = RawRequest {
            method: HttpMethod::Get,
            path: path.to_string(),
            query: Vec::new(),
            headers,
            body: Vec::new(),
        };
        let resp = self.dispatch(request).await?;
        Ok(resp.body)
    }

    async fn put_bytes(&self, path: &str, body: &[u8]) -> Result<PutBlobResponse, ServerSyncError> {
        let mut headers = self.auth_headers(true)?;
        headers.push((
            "content-type".to_string(),
            "application/octet-stream".to_string(),
        ));
        let resp = self
            .send_expecting(HttpMethod::Put, path, &headers, body.to_vec(), true)
            .await?;
        decode_json(&resp.body)
    }

    /// Dispatch a request and convert non-2xx responses into errors.
    async fn send_expecting(
        &self,
        method: HttpMethod,
        path: &str,
        headers: &[(String, String)],
        body: Vec<u8>,
        auth: bool,
    ) -> Result<RawResponse, ServerSyncError> {
        let mut all_headers = self.auth_headers(auth)?;
        for h in headers {
            // Avoid duplicating the auth header already added above.
            if !h.0.eq_ignore_ascii_case("authorization") {
                all_headers.push(h.clone());
            }
        }
        let request = RawRequest {
            method,
            path: path.to_string(),
            query: Vec::new(),
            headers: all_headers,
            body,
        };
        self.dispatch(request).await
    }

    async fn dispatch(&self, request: RawRequest) -> Result<RawResponse, ServerSyncError> {
        let resp = self.sender.send(request).await?;
        if resp.is_success() {
            return Ok(resp);
        }
        let message = parse_error_message(&resp.body);
        Err(ServerSyncError::Http {
            status: resp.status,
            message,
        })
    }
}

fn decode_json<Res: serde::de::DeserializeOwned>(body: &[u8]) -> Result<Res, ServerSyncError> {
    serde_json::from_slice(body).map_err(|e| ServerSyncError::Decode(e.to_string()))
}

/// Best-effort extraction of an error message from a non-2xx body.
fn parse_error_message(body: &[u8]) -> String {
    if let Ok(err) = serde_json::from_slice::<ErrorResponse>(body) {
        return err.error;
    }
    String::from_utf8_lossy(body).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::future::Future;

    /// Minimal executor for the always-ready futures the mock sender produces.
    fn block_on<F: Future>(future: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll};
        let mut fut = pin!(future);
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    /// Records requests and replays a queue of canned responses.
    struct MockSender {
        responses: RefCell<VecDeque<RawResponse>>,
        requests: RefCell<Vec<RawRequest>>,
    }

    impl MockSender {
        fn new(responses: Vec<RawResponse>) -> Self {
            Self {
                responses: RefCell::new(responses.into()),
                requests: RefCell::new(Vec::new()),
            }
        }

        fn last_request(&self) -> RawRequest {
            self.requests.borrow().last().cloned().expect("a request")
        }
    }

    impl RawHttpSender for MockSender {
        async fn send(&self, request: RawRequest) -> Result<RawResponse, ServerSyncError> {
            self.requests.borrow_mut().push(request);
            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| ServerSyncError::Transport("no canned response".into()))
        }
    }

    fn json_response(status: u16, value: &serde_json::Value) -> RawResponse {
        RawResponse {
            status,
            body: serde_json::to_vec(value).unwrap(),
        }
    }

    #[test]
    fn register_encodes_request_and_decodes_response() {
        let sender = MockSender::new(vec![json_response(
            201,
            &serde_json::json!({ "user_id": "user-123" }),
        )]);
        let client = ServerSyncClient::new(sender);

        let resp = block_on(client.register("alice", b"pw")).expect("register");
        assert_eq!(resp.user_id, "user-123");

        let req = client.sender.last_request();
        assert_eq!(req.method, HttpMethod::Post);
        assert_eq!(req.path, "/api/v1/auth/register");
        let body: RegisterRequest = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body.username, "alice");
        // Salt and verifier are non-empty hex.
        assert!(!body.salt.is_empty());
        assert!(!body.verifier.is_empty());
        assert!(hex_decode(&body.salt).is_ok());
        assert!(hex_decode(&body.verifier).is_ok());
        // content-type set for JSON.
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k == "content-type" && v == "application/json")
        );
    }

    #[test]
    fn http_error_is_classified() {
        let sender = MockSender::new(vec![json_response(
            400,
            &serde_json::json!({ "error": "username taken" }),
        )]);
        let client = ServerSyncClient::new(sender);
        let err = block_on(client.register("alice", b"pw")).unwrap_err();
        assert_eq!(
            err,
            ServerSyncError::Http {
                status: 400,
                message: "username taken".into(),
            }
        );
    }

    #[test]
    fn authenticated_call_requires_token() {
        let sender = MockSender::new(vec![]);
        let client = ServerSyncClient::new(sender);
        let err = block_on(client.create_vault("v1")).unwrap_err();
        assert_eq!(err, ServerSyncError::NotAuthenticated);
    }

    #[test]
    fn with_token_sets_bearer_header() {
        let sender = MockSender::new(vec![json_response(
            201,
            &serde_json::json!({ "id": "v1", "created_at": "t" }),
        )]);
        let client = ServerSyncClient::with_token(sender, "tok-abc".into());
        let resp = block_on(client.create_vault("v1")).expect("create");
        assert_eq!(resp.id, "v1");

        let req = client.sender.last_request();
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k == "authorization" && v == "Bearer tok-abc")
        );
    }

    #[test]
    fn login_rejects_bad_server_proof() {
        // init returns a valid-looking salt + non-zero B; verify returns a bogus
        // proof so the client must reject it and stay unauthenticated.
        let big_b = "02".repeat(256); // 256 bytes, non-zero mod N
        let init = json_response(
            200,
            &serde_json::json!({
                "handshake_id": "h1",
                "salt": "00112233445566778899aabbccddeeff",
                "server_public": big_b,
            }),
        );
        let verify = json_response(
            200,
            &serde_json::json!({
                "server_proof": "deadbeef",
                "token": "should-not-be-stored",
            }),
        );
        let sender = MockSender::new(vec![init, verify]);
        let mut client = ServerSyncClient::new(sender);

        let err = block_on(client.login("alice", b"pw")).unwrap_err();
        assert_eq!(err, ServerSyncError::ProofMismatch);
        assert!(!client.is_authenticated());
        assert!(client.token().is_none());
    }

    #[test]
    fn put_and_get_batch_round_trip() {
        let put = json_response(
            201,
            &serde_json::json!({
                "path": "v1/batches/d1/b1.enc",
                "size": 4,
                "content_hash": "abcd",
            }),
        );
        let get = RawResponse {
            status: 200,
            body: vec![1, 2, 3, 4],
        };
        let sender = MockSender::new(vec![put, get]);
        let client = ServerSyncClient::with_token(sender, "tok".into());

        let meta = block_on(client.put_batch("v1", "d1", "b1", &[1, 2, 3, 4])).expect("put");
        assert_eq!(meta.size, 4);
        let put_req = client.sender.last_request();
        assert_eq!(put_req.method, HttpMethod::Put);
        assert_eq!(put_req.path, "/api/v1/vaults/v1/batches/d1/b1");
        assert_eq!(put_req.body, vec![1, 2, 3, 4]);

        let bytes = block_on(client.get_batch("v1", "d1", "b1")).expect("get");
        assert_eq!(bytes, vec![1, 2, 3, 4]);
    }

    #[test]
    fn list_remote_batches_parses_and_filters() {
        let resp = json_response(
            200,
            &serde_json::json!({
                "entries": [
                    { "path": "v1/batches/d1/b1.enc", "size": 10, "content_hash": "aa", "created_at": "t1" },
                    { "path": "v1/garbage/nope.enc", "size": 5, "content_hash": "bb", "created_at": "t2" }
                ],
                "has_more": false,
            }),
        );
        let sender = MockSender::new(vec![resp]);
        let client = ServerSyncClient::with_token(sender, "tok".into());
        let metas =
            block_on(client.list_remote_batches("v1", &ListBatchesQuery::default())).expect("list");
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].batch_id, "b1");
        assert_eq!(metas[0].device_id, "d1");
        assert_eq!(metas[0].size, 10);
    }

    #[test]
    fn list_remote_batches_page_exposes_cursor() {
        let resp = json_response(
            200,
            &serde_json::json!({
                "entries": [
                    { "path": "v1/batches/d1/b1.enc", "size": 10, "content_hash": "aa", "created_at": "t1" }
                ],
                "has_more": true,
                "cursor": "t1|v1/batches/d1/b1.enc",
            }),
        );
        let sender = MockSender::new(vec![resp]);
        let client = ServerSyncClient::with_token(sender, "tok".into());
        let page = block_on(client.list_remote_batches_page("v1", &ListBatchesQuery::default()))
            .expect("list page");
        assert_eq!(page.metas.len(), 1);
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some("t1|v1/batches/d1/b1.enc"));
    }

    #[test]
    fn list_remote_batches_page_defaults_cursor_when_absent() {
        // Older servers omit `cursor`; it must deserialize to None.
        let resp = json_response(
            200,
            &serde_json::json!({
                "entries": [],
                "has_more": false,
            }),
        );
        let sender = MockSender::new(vec![resp]);
        let client = ServerSyncClient::with_token(sender, "tok".into());
        let page = block_on(client.list_remote_batches_page("v1", &ListBatchesQuery::default()))
            .expect("list page");
        assert!(!page.has_more);
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn list_query_params_are_forwarded() {
        let resp = json_response(
            200,
            &serde_json::json!({ "entries": [], "has_more": false }),
        );
        let sender = MockSender::new(vec![resp]);
        let client = ServerSyncClient::with_token(sender, "tok".into());
        let query = ListBatchesQuery {
            since: Some("2024".into()),
            device_id: Some("d1".into()),
            limit: Some(25),
        };
        let _ = block_on(client.list_batches("v1", &query)).expect("list");
        let req = client.sender.last_request();
        assert!(
            req.query
                .contains(&("since".to_string(), "2024".to_string()))
        );
        assert!(
            req.query
                .contains(&("device_id".to_string(), "d1".to_string()))
        );
        assert!(req.query.contains(&("limit".to_string(), "25".to_string())));
    }
}
