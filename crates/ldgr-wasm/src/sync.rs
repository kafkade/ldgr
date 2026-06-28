//! WASM sync surface (behind the `sync` feature).
//!
//! Two pieces, both keeping `ldgr-core`'s zero-I/O contract intact:
//!
//! - [`merge_batch`]: a thin wrapper over [`ldgr_core::sync::merge_events`] so
//!   the canonical conflict policy runs in Rust. The per-row version gate stays
//!   in TypeScript because it needs the local sql.js row version.
//! - [`WasmSyncClient`]: wraps the transport-agnostic
//!   [`ServerSyncClient`](ldgr_core::sync::server::ServerSyncClient) over a
//!   [`JsFetchSender`] that performs the actual HTTP via an injected JS callback
//!   (`fetch`). No networking happens in Rust — the callback owns it.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Object, Promise, Reflect, Uint8Array};
use serde::Serialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use ldgr_core::sync::conflicts::SyncConflict;
use ldgr_core::sync::events::{EventBatch, SyncEvent, VectorClock};
use ldgr_core::sync::server::{
    ListBatchesQuery, RawHttpSender, RawRequest, RawResponse, ServerSyncClient, ServerSyncError,
};

// ── Merge ────────────────────────────────────────────────────────────────────

/// Serializable view of [`ldgr_core::sync::conflicts::MergeResult`]
/// (`MergeResult` itself is not `Serialize`).
#[derive(Serialize)]
struct MergeOutput {
    applied: Vec<SyncEvent>,
    conflicts: Vec<SyncConflict>,
    skipped: usize,
}

/// Three-way merge a downloaded remote batch against local pending events using
/// the canonical [`ldgr_core::sync::merge_events`] policy.
///
/// Inputs are JSON strings:
/// - `local_pending_json`: `SyncEvent[]` not yet synced locally.
/// - `remote_batch_json`: the decrypted `EventBatch` (from `openBatch`).
/// - `local_clock_json`: the local `VectorClock`.
/// - `now`: RFC3339 timestamp used to stamp detected conflicts.
///
/// Returns JSON `{ applied: SyncEvent[], conflicts: SyncConflict[], skipped: number }`.
/// The caller applies `applied` to sql.js (with a per-row version check) and
/// persists `conflicts` for review.
#[wasm_bindgen(js_name = mergeBatch)]
pub fn merge_batch(
    local_pending_json: &str,
    remote_batch_json: &str,
    local_clock_json: &str,
    now: &str,
) -> Result<String, JsError> {
    let local_pending: Vec<SyncEvent> = serde_json::from_str(local_pending_json)
        .map_err(|e| JsError::new(&format!("invalid local pending JSON: {e}")))?;
    let remote_batch: EventBatch = serde_json::from_str(remote_batch_json)
        .map_err(|e| JsError::new(&format!("invalid remote batch JSON: {e}")))?;
    let local_clock: VectorClock = serde_json::from_str(local_clock_json)
        .map_err(|e| JsError::new(&format!("invalid local clock JSON: {e}")))?;

    let result = ldgr_core::sync::merge_events(
        &local_pending,
        &remote_batch.events,
        &local_clock,
        &remote_batch.vector_clock,
        now,
    );

    let out = MergeOutput {
        applied: result.applied,
        conflicts: result.conflicts,
        skipped: result.skipped,
    };
    serde_json::to_string(&out).map_err(|e| JsError::new(&format!("serialization error: {e}")))
}

// ── JS fetch sender ────────────────────────────────────────────────────────────

/// A [`RawHttpSender`] backed by an injected JavaScript async callback.
///
/// The callback receives a plain request object
/// `{ method, path, query, headers, body }` and must return a `Promise` of
/// `{ status: number, body: Uint8Array }`. All real network I/O lives in JS,
/// preserving the no-networking-in-Rust boundary.
#[derive(Clone)]
struct JsFetchSender {
    callback: js_sys::Function,
}

impl JsFetchSender {
    fn build_request_object(request: &RawRequest) -> Result<Object, ServerSyncError> {
        let obj = Object::new();
        let set = |key: &str, val: &JsValue| -> Result<(), ServerSyncError> {
            Reflect::set(&obj, &JsValue::from_str(key), val)
                .map(|_| ())
                .map_err(|e| ServerSyncError::Transport(format!("failed to build request: {e:?}")))
        };

        set("method", &JsValue::from_str(request.method.as_str()))?;
        set("path", &JsValue::from_str(&request.path))?;

        let query = js_sys::Array::new();
        for (k, v) in &request.query {
            let pair = js_sys::Array::new();
            pair.push(&JsValue::from_str(k));
            pair.push(&JsValue::from_str(v));
            query.push(&pair);
        }
        set("query", &query)?;

        let headers = js_sys::Array::new();
        for (k, v) in &request.headers {
            let pair = js_sys::Array::new();
            pair.push(&JsValue::from_str(k));
            pair.push(&JsValue::from_str(v));
            headers.push(&pair);
        }
        set("headers", &headers)?;

        set("body", &Uint8Array::from(request.body.as_slice()))?;

        Ok(obj)
    }
}

impl RawHttpSender for JsFetchSender {
    async fn send(&self, request: RawRequest) -> Result<RawResponse, ServerSyncError> {
        let obj = Self::build_request_object(&request)?;

        let ret = self
            .callback
            .call1(&JsValue::NULL, &obj)
            .map_err(|e| ServerSyncError::Transport(format!("fetch callback threw: {e:?}")))?;

        let promise: Promise = ret.dyn_into().map_err(|_| {
            ServerSyncError::Transport("fetch callback did not return a Promise".to_string())
        })?;

        let resolved = JsFuture::from(promise)
            .await
            .map_err(|e| ServerSyncError::Transport(format!("fetch failed: {e:?}")))?;

        let status_val = Reflect::get(&resolved, &JsValue::from_str("status"))
            .map_err(|e| ServerSyncError::Transport(format!("missing status: {e:?}")))?;
        let status = status_val
            .as_f64()
            .ok_or_else(|| ServerSyncError::Transport("status is not a number".to_string()))?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let status = status as u16;

        let body_val = Reflect::get(&resolved, &JsValue::from_str("body"))
            .map_err(|e| ServerSyncError::Transport(format!("missing body: {e:?}")))?;
        let body = if body_val.is_undefined() || body_val.is_null() {
            Vec::new()
        } else {
            Uint8Array::new(&body_val).to_vec()
        };

        Ok(RawResponse { status, body })
    }
}

// ── Client ─────────────────────────────────────────────────────────────────────

fn sync_err(e: ServerSyncError) -> JsError {
    JsError::new(&format!("sync error: {e}"))
}

/// Browser-side `ldgr-server` sync client.
///
/// Holds the injected JS `fetch` callback and an in-memory bearer token. A fresh
/// [`ServerSyncClient`] is constructed per call from the cached token, so no
/// borrow is held across `await` (keeps the exported futures `'static`).
#[wasm_bindgen]
pub struct WasmSyncClient {
    sender: JsFetchSender,
    token: Rc<RefCell<Option<String>>>,
}

#[wasm_bindgen]
impl WasmSyncClient {
    /// Create a client. `send_callback` is a JS function
    /// `(request) => Promise<{ status, body }>` that performs the HTTP request.
    #[wasm_bindgen(constructor)]
    pub fn new(send_callback: js_sys::Function) -> WasmSyncClient {
        WasmSyncClient {
            sender: JsFetchSender {
                callback: send_callback,
            },
            token: Rc::new(RefCell::new(None)),
        }
    }

    /// Create a client that resumes a previously obtained session token.
    #[wasm_bindgen(js_name = withToken)]
    pub fn with_token(send_callback: js_sys::Function, token: String) -> WasmSyncClient {
        WasmSyncClient {
            sender: JsFetchSender {
                callback: send_callback,
            },
            token: Rc::new(RefCell::new(Some(token))),
        }
    }

    /// The current bearer token, if authenticated.
    #[wasm_bindgen(getter)]
    pub fn token(&self) -> Option<String> {
        self.token.borrow().clone()
    }

    /// Whether the client holds a session token.
    #[wasm_bindgen(js_name = isAuthenticated)]
    pub fn is_authenticated(&self) -> bool {
        self.token.borrow().is_some()
    }

    /// Drop the current session token.
    #[wasm_bindgen]
    pub fn logout(&self) {
        *self.token.borrow_mut() = None;
    }

    fn client(&self) -> ServerSyncClient<JsFetchSender> {
        match self.token.borrow().clone() {
            Some(t) => ServerSyncClient::with_token(self.sender.clone(), t),
            None => ServerSyncClient::new(self.sender.clone()),
        }
    }

    /// Register a new account (SRP-6a). The password never leaves the browser —
    /// only `(salt, verifier)` is sent.
    #[wasm_bindgen]
    pub async fn register(&self, username: String, password: String) -> Result<(), JsError> {
        let client = self.client();
        client
            .register(&username, password.as_bytes())
            .await
            .map(|_| ())
            .map_err(sync_err)
    }

    /// Perform the SRP-6a login handshake and cache the session token.
    #[wasm_bindgen]
    pub async fn login(&self, username: String, password: String) -> Result<(), JsError> {
        let mut client = self.client();
        client
            .login(&username, password.as_bytes())
            .await
            .map_err(sync_err)?;
        *self.token.borrow_mut() = client.token().map(str::to_string);
        Ok(())
    }

    /// Create a remote vault. Safe to call repeatedly; the server rejects
    /// duplicates, which the caller may treat as "already exists".
    #[wasm_bindgen(js_name = createVault)]
    pub async fn create_vault(&self, vault_id: String) -> Result<(), JsError> {
        let client = self.client();
        client
            .create_vault(&vault_id)
            .await
            .map(|_| ())
            .map_err(sync_err)
    }

    /// Upload an encrypted event batch (put-if-absent).
    #[wasm_bindgen(js_name = putBatch)]
    pub async fn put_batch(
        &self,
        vault_id: String,
        device_id: String,
        batch_id: String,
        ciphertext: Vec<u8>,
    ) -> Result<(), JsError> {
        let client = self.client();
        client
            .put_batch(&vault_id, &device_id, &batch_id, &ciphertext)
            .await
            .map(|_| ())
            .map_err(sync_err)
    }

    /// Download an encrypted event batch.
    #[wasm_bindgen(js_name = getBatch)]
    pub async fn get_batch(
        &self,
        vault_id: String,
        device_id: String,
        batch_id: String,
    ) -> Result<Vec<u8>, JsError> {
        let client = self.client();
        client
            .get_batch(&vault_id, &device_id, &batch_id)
            .await
            .map_err(sync_err)
    }

    /// List remote batch blobs, returning canonical
    /// [`RemoteBatchMeta`](ldgr_core::sync::RemoteBatchMeta) records as a JSON
    /// array. Paths that don't match the batch layout are skipped.
    #[wasm_bindgen(js_name = listBatches)]
    pub async fn list_batches(
        &self,
        vault_id: String,
        since: Option<String>,
        device_id: Option<String>,
        limit: Option<u32>,
    ) -> Result<String, JsError> {
        let client = self.client();
        let query = ListBatchesQuery {
            since,
            device_id,
            limit,
        };
        let metas = client
            .list_remote_batches(&vault_id, &query)
            .await
            .map_err(sync_err)?;
        serde_json::to_string(&metas)
            .map_err(|e| JsError::new(&format!("serialization error: {e}")))
    }
}
