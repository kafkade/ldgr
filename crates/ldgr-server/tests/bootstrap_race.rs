//! Concurrency regression tests for the **atomic first-admin bootstrap**
//! (issue #297, finding F7).
//!
//! The old election read `count_users() == 0` and then, in a *separate* DB call,
//! inserted the user — a check-then-insert race where two concurrent first-time
//! registrations could both become `admin`. These tests spawn many simultaneous
//! `register` calls against a fresh, no-`LDGR_ADMIN_EMAIL` server (each with a
//! distinct email, so the `UNIQUE(email)` index cannot mask the race) and assert
//! that **exactly one** admin is ever elected.
//!
//! Requests are dispatched straight into the axum router via `tower`'s
//! `oneshot`; the SRP `(salt, verifier)` come from the real `ldgr-core` client
//! primitives.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use ldgr_core::sync::server::register_with_salt;
use ldgr_server::auth::hex_encode;
use ldgr_server::config::{Config, RegistrationPolicy};
use ldgr_server::{api, auth, state, storage};

fn config(policy: RegistrationPolicy) -> Config {
    Config {
        bind_addr: "127.0.0.1:8080".parse().unwrap(),
        db_path: ":memory:".into(),
        session_ttl_hours: 720,
        relay_ttl_minutes: 10,
        max_blob_bytes: 52_428_800,
        srp_handshake_ttl_secs: 120,
        registration_policy: policy,
        admin_email: None,
        allowed_origins: Vec::new(),
        default_user_quota_bytes: 1_073_741_824,
        server_name: "race-test".into(),
    }
}

/// Boot a fresh in-memory server (empty DB) with the given registration policy
/// and no configured admin email, so the first-user fallback election is active.
fn fresh_state(policy: RegistrationPolicy) -> state::SharedState {
    let db = storage::ServerDb::open(":memory:").expect("open in-memory db");
    let config = config(policy);
    let srp_ttl = std::time::Duration::from_secs(config.srp_handshake_ttl_secs);
    Arc::new(state::AppState {
        db,
        srp_handshakes: auth::srp::SrpHandshakeStore::new(srp_ttl),
        config,
    })
}

/// Build a self-contained `register` request for `username` with a distinct
/// per-user email.
fn register_request(username: &str) -> Request<Body> {
    let email = format!("{username}@example.org");
    let reg = register_with_salt(username, b"pw", vec![0x5a; 16]);
    let body = json!({
        "username": username,
        "email": email,
        "salt": hex_encode(&reg.salt),
        "verifier": hex_encode(&reg.verifier),
    });
    Request::builder()
        .method("POST")
        .uri("/api/v1/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

async fn dispatch(state: state::SharedState, req: Request<Body>) -> (StatusCode, Value) {
    let resp = api::router(state)
        .oneshot(req)
        .await
        .expect("router oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

/// Fire `n` `register` calls concurrently against the same shared state and
/// return each `(status, role)` outcome (`role` is `None` when the response
/// carried no role, e.g. a rejection).
async fn register_concurrently(
    state: &state::SharedState,
    n: usize,
) -> Vec<(StatusCode, Option<String>)> {
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let state = state.clone();
        let req = register_request(&format!("user{i}"));
        handles.push(tokio::spawn(async move { dispatch(state, req).await }));
    }
    let mut out = Vec::with_capacity(n);
    for h in handles {
        let (st, body) = h.await.expect("task join");
        let role = body.get("role").and_then(Value::as_str).map(str::to_string);
        out.push((st, role));
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_registration_elects_exactly_one_admin_under_open_policy() {
    // Repeat to shake out interleavings.
    for round in 0..10 {
        let state = fresh_state(RegistrationPolicy::Open);
        let results = register_concurrently(&state, 16).await;

        let admins = results
            .iter()
            .filter(|(_, r)| r.as_deref() == Some("admin"))
            .count();
        let users = results
            .iter()
            .filter(|(_, r)| r.as_deref() == Some("user"))
            .count();

        assert_eq!(
            admins, 1,
            "round {round}: exactly one admin must be elected, got {admins} in {results:?}"
        );
        assert_eq!(
            users, 15,
            "round {round}: the rest must be normal users, got {users} in {results:?}"
        );
        assert!(
            results.iter().all(|(st, _)| *st == StatusCode::CREATED),
            "round {round}: every open-policy registration should succeed: {results:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_registration_elects_exactly_one_admin_under_invite_only() {
    for round in 0..10 {
        let state = fresh_state(RegistrationPolicy::InviteOnly);
        let results = register_concurrently(&state, 16).await;

        let admins = results
            .iter()
            .filter(|(_, r)| r.as_deref() == Some("admin"))
            .count();
        let rejected = results
            .iter()
            .filter(|(st, _)| *st == StatusCode::FORBIDDEN)
            .count();

        assert_eq!(
            admins, 1,
            "round {round}: exactly one admin must be elected, got {admins} in {results:?}"
        );
        assert_eq!(
            rejected, 15,
            "round {round}: non-winners must be rejected under invite-only, got {rejected} in {results:?}"
        );
    }
}

#[tokio::test]
async fn sequential_first_user_is_admin_and_second_is_user_under_open() {
    let state = fresh_state(RegistrationPolicy::Open);

    let (s1, b1) = dispatch(state.clone(), register_request("alice")).await;
    assert_eq!(s1, StatusCode::CREATED);
    assert_eq!(b1["role"], "admin");

    let (s2, b2) = dispatch(state.clone(), register_request("bob")).await;
    assert_eq!(s2, StatusCode::CREATED);
    assert_eq!(b2["role"], "user");
}

#[tokio::test]
async fn sequential_second_user_is_rejected_under_invite_only() {
    let state = fresh_state(RegistrationPolicy::InviteOnly);

    let (s1, b1) = dispatch(state.clone(), register_request("alice")).await;
    assert_eq!(s1, StatusCode::CREATED, "first user bootstraps as admin");
    assert_eq!(b1["role"], "admin");

    let (s2, _b2) = dispatch(state.clone(), register_request("bob")).await;
    assert_eq!(
        s2,
        StatusCode::FORBIDDEN,
        "a second user without an invite is refused under invite-only"
    );
}
