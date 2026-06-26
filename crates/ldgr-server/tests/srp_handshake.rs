//! In-process integration test: drives the real `ldgr-server` router with the
//! `ldgr-core` SRP-6a client, asserting the published server params accept a
//! client-generated registration verifier and login handshake.
//!
//! No sockets are bound — requests are dispatched through the router via
//! `tower`'s `oneshot`, so this is a true in-process test.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

use ldgr_core::sync::server::{
    HttpMethod, ListBatchesQuery, RawHttpSender, RawRequest, RawResponse, ServerSyncClient,
    ServerSyncError,
};
use ldgr_server::{api, auth, config, state, storage};

/// A [`RawHttpSender`] that dispatches requests straight into the axum router.
struct RouterTransport {
    router: Router,
}

impl RouterTransport {
    fn new() -> Self {
        // Single in-memory SQLite connection backs the whole app for the test.
        let db = storage::ServerDb::open(":memory:").expect("open in-memory db");
        let cfg = config::Config::from_env();
        let srp_ttl = std::time::Duration::from_secs(cfg.srp_handshake_ttl_secs);
        let app_state = Arc::new(state::AppState {
            db,
            srp_handshakes: auth::srp::SrpHandshakeStore::new(srp_ttl),
            config: cfg,
        });
        Self {
            router: api::router(app_state),
        }
    }
}

fn method_str(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Delete => "DELETE",
    }
}

impl RawHttpSender for RouterTransport {
    async fn send(&self, request: RawRequest) -> Result<RawResponse, ServerSyncError> {
        // Build the URI (path + optional query string).
        let mut uri = request.path.clone();
        if !request.query.is_empty() {
            let qs: Vec<String> = request
                .query
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            uri.push('?');
            uri.push_str(&qs.join("&"));
        }

        let mut builder = Request::builder()
            .method(method_str(request.method))
            .uri(uri);
        for (k, v) in &request.headers {
            builder = builder.header(k, v);
        }
        let http_req = builder
            .body(Body::from(request.body))
            .map_err(|e| ServerSyncError::Transport(e.to_string()))?;

        let response = self
            .router
            .clone()
            .oneshot(http_req)
            .await
            .map_err(|e| ServerSyncError::Transport(e.to_string()))?;

        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|e| ServerSyncError::Transport(e.to_string()))?;

        Ok(RawResponse {
            status,
            body: bytes.to_vec(),
        })
    }
}

#[tokio::test]
async fn register_and_login_handshake_is_accepted() {
    let mut client = ServerSyncClient::new(RouterTransport::new());

    // Registration: the client derives the verifier; the server stores it.
    let reg = client
        .register("alice", b"correct horse battery staple")
        .await
        .expect("register should succeed");
    assert!(!reg.user_id.is_empty());

    // Login: full SRP-6a init/verify handshake against the real server.
    client
        .login("alice", b"correct horse battery staple")
        .await
        .expect("login should succeed");

    assert!(client.is_authenticated());
    assert!(client.token().is_some());
    // The SRP session key was derived and retained.
    assert_eq!(client.session_key().map(<[u8]>::len), Some(32));
}

#[tokio::test]
async fn login_with_wrong_password_is_rejected() {
    let mut client = ServerSyncClient::new(RouterTransport::new());
    client.register("bob", b"hunter2").await.expect("register");

    let err = client
        .login("bob", b"not-the-password")
        .await
        .expect_err("login must fail");

    match err {
        ServerSyncError::Http { status, .. } => assert_eq!(status, 401),
        other => panic!("expected 401, got {other:?}"),
    }
    assert!(!client.is_authenticated());
}

#[tokio::test]
async fn authenticated_vault_and_batch_flow() {
    let mut client = ServerSyncClient::new(RouterTransport::new());
    client.register("carol", b"s3cr3t").await.expect("register");
    client.login("carol", b"s3cr3t").await.expect("login");

    // Create a vault, then list it back.
    let vault = client.create_vault("vault-1").await.expect("create vault");
    assert_eq!(vault.id, "vault-1");

    let vaults = client.list_vaults().await.expect("list vaults");
    assert!(vaults.iter().any(|v| v.id == "vault-1"));

    // Push an encrypted batch blob and pull it back byte-for-byte.
    let ciphertext = b"encrypted-batch-bytes";
    let put = client
        .put_batch("vault-1", "device-a", "batch-1", ciphertext)
        .await
        .expect("put batch");
    assert_eq!(put.size, i64::try_from(ciphertext.len()).unwrap());

    let pulled = client
        .get_batch("vault-1", "device-a", "batch-1")
        .await
        .expect("get batch");
    assert_eq!(pulled, ciphertext);

    // The batch shows up in a parsed remote listing.
    let metas = client
        .list_remote_batches("vault-1", &ListBatchesQuery::default())
        .await
        .expect("list remote batches");
    assert!(
        metas
            .iter()
            .any(|m| m.batch_id == "batch-1" && m.device_id == "device-a")
    );
}

#[tokio::test]
async fn unauthenticated_request_without_token_fails_locally() {
    let client = ServerSyncClient::new(RouterTransport::new());
    let err = client
        .create_vault("vault-x")
        .await
        .expect_err("should fail");
    assert!(matches!(err, ServerSyncError::NotAuthenticated));
}
