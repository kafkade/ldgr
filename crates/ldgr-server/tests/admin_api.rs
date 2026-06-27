//! In-process integration tests for the ADR-008 admin API (#176): the admin
//! authorization guard, user management (list/create/disable/enable/delete,
//! role + quota changes), last-admin protection, invite issuance/listing/revoke
//! with end-to-end redemption through the existing `register` path, runtime
//! server settings (consulted by registration + quota logic), and stats.
//!
//! Requests are dispatched straight into the axum router via `tower`'s
//! `oneshot` — no sockets bound. SRP `(salt, verifier)` and login proofs come
//! from the real `ldgr-core` client primitives.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use ldgr_core::sync::server::{ClientLogin, register_with_salt};
use ldgr_server::auth::hex_encode;
use ldgr_server::config::{Config, RegistrationPolicy};
use ldgr_server::{api, auth, state, storage};

// ── Harness ────────────────────────────────────────────────────────────────────

struct Harness {
    state: state::SharedState,
}

fn base_config() -> Config {
    Config {
        bind_addr: "127.0.0.1:8080".parse().unwrap(),
        db_path: ":memory:".into(),
        session_ttl_hours: 720,
        relay_ttl_minutes: 10,
        max_blob_bytes: 52_428_800,
        srp_handshake_ttl_secs: 120,
        registration_policy: RegistrationPolicy::Open,
        admin_email: None,
        default_user_quota_bytes: 1_073_741_824,
        server_name: "ldgr-server".into(),
    }
}

impl Harness {
    fn with_config(config: Config) -> Self {
        let db = storage::ServerDb::open(":memory:").expect("open in-memory db");
        let srp_ttl = std::time::Duration::from_secs(config.srp_handshake_ttl_secs);
        let app_state = Arc::new(state::AppState {
            db,
            srp_handshakes: auth::srp::SrpHandshakeStore::new(srp_ttl),
            config,
        });
        Self { state: app_state }
    }

    fn router(&self) -> Router {
        api::router(self.state.clone())
    }

    async fn send(&self, req: Request<Body>) -> (StatusCode, Value) {
        let resp = self.router().oneshot(req).await.expect("router oneshot");
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

    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        token: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        let body = match body {
            Some(v) => Body::from(serde_json::to_vec(v).unwrap()),
            None => Body::empty(),
        };
        self.send(builder.body(body).unwrap()).await
    }

    async fn get(&self, path: &str, token: Option<&str>) -> (StatusCode, Value) {
        self.request("GET", path, None, token).await
    }

    async fn post(&self, path: &str, body: &Value, token: Option<&str>) -> (StatusCode, Value) {
        self.request("POST", path, Some(body), token).await
    }

    async fn patch(&self, path: &str, body: &Value, token: Option<&str>) -> (StatusCode, Value) {
        self.request("PATCH", path, Some(body), token).await
    }

    async fn delete(&self, path: &str, token: Option<&str>) -> (StatusCode, Value) {
        self.request("DELETE", path, None, token).await
    }

    /// Register `username`/`password` with optional email + invite token.
    async fn register(
        &self,
        username: &str,
        password: &[u8],
        email: Option<&str>,
        invite_token: Option<&str>,
    ) -> (StatusCode, Value) {
        let reg = register_with_salt(username, password, vec![0x5a; 16]);
        let mut body = json!({
            "username": username,
            "salt": hex_encode(&reg.salt),
            "verifier": hex_encode(&reg.verifier),
        });
        if let Some(e) = email {
            body["email"] = json!(e);
        }
        if let Some(t) = invite_token {
            body["invite_token"] = json!(t);
        }
        self.post("/api/v1/auth/register", &body, None).await
    }

    /// Full SRP-6a login handshake; returns the bearer token.
    async fn login(&self, username: &str, password: &[u8]) -> Result<String, StatusCode> {
        let (login, a_pub) = ClientLogin::start(username, password);
        let (st, init) = self
            .post(
                "/api/v1/auth/login/init",
                &json!({ "username": username, "client_public": hex_encode(&a_pub) }),
                None,
            )
            .await;
        if st != StatusCode::OK {
            return Err(st);
        }
        let handshake_id = init["handshake_id"].as_str().unwrap().to_string();
        let salt = hex_decode(init["salt"].as_str().unwrap());
        let server_public = hex_decode(init["server_public"].as_str().unwrap());

        let session = login.finish(&salt, &server_public).expect("client finish");
        let (st, verify) = self
            .post(
                "/api/v1/auth/login/verify",
                &json!({
                    "handshake_id": handshake_id,
                    "client_proof": hex_encode(session.proof()),
                }),
                None,
            )
            .await;
        if st != StatusCode::OK {
            return Err(st);
        }
        Ok(verify["token"].as_str().unwrap().to_string())
    }

    /// Bootstrap an admin (first user under Open policy) and return its token.
    async fn bootstrap_admin(&self) -> String {
        let (st, body) = self
            .register("admin", b"pw-admin", Some("admin@example.org"), None)
            .await;
        assert_eq!(st, StatusCode::CREATED);
        assert_eq!(body["role"], "admin");
        self.login("admin", b"pw-admin").await.expect("admin login")
    }

    async fn put_blob(
        &self,
        vault: &str,
        batch: &str,
        body: &[u8],
        token: &str,
    ) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/vaults/{vault}/batches/dev/{batch}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_vec()))
            .unwrap();
        self.send(req).await
    }
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Salt+verifier for an admin-created account (`POST /users`).
fn srp_material(username: &str, password: &[u8]) -> (String, String) {
    let reg = register_with_salt(username, password, vec![0x5a; 16]);
    (hex_encode(&reg.salt), hex_encode(&reg.verifier))
}

// ── Admin authorization guard ────────────────────────────────────────────────

#[tokio::test]
async fn admin_routes_reject_non_admins_and_anonymous() {
    let h = Harness::with_config(base_config());
    let _admin = h.bootstrap_admin().await;

    // A second (regular) user.
    let (st, _) = h
        .register("bob", b"pw-bob", Some("bob@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let user_token = h.login("bob", b"pw-bob").await.expect("bob login");

    // No token → 401.
    let (st, _) = h.get("/api/v1/admin/users", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // Valid non-admin token → 403 on every admin surface.
    for (method, path) in [
        ("GET", "/api/v1/admin/users"),
        ("GET", "/api/v1/admin/invites"),
        ("GET", "/api/v1/admin/settings"),
        ("GET", "/api/v1/admin/stats"),
    ] {
        let (st, _) = h.request(method, path, None, Some(&user_token)).await;
        assert_eq!(
            st,
            StatusCode::FORBIDDEN,
            "{method} {path} must be admin-only"
        );
    }
    let (st, _) = h
        .post(
            "/api/v1/admin/invites",
            &json!({ "role": "user" }),
            Some(&user_token),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

// ── User management ──────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_lists_creates_disables_enables_and_deletes_users() {
    let h = Harness::with_config(base_config());
    let admin = h.bootstrap_admin().await;

    // List: only the admin so far.
    let (st, users) = h.get("/api/v1/admin/users", Some(&admin)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(users.as_array().unwrap().len(), 1);

    // Create an account via the admin API.
    let (salt, verifier) = srp_material("carol", b"pw-carol");
    let (st, created) = h
        .post(
            "/api/v1/admin/users",
            &json!({
                "username": "carol",
                "salt": salt,
                "verifier": verifier,
                "email": "carol@example.org",
                "role": "user",
                "quota_bytes": 5000,
            }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(created["role"], "user");
    assert_eq!(created["quota_bytes"], 5000);
    let carol_id = created["id"].as_str().unwrap().to_string();

    // The admin-created account can log in with its password (SRP round-trips).
    assert!(h.login("carol", b"pw-carol").await.is_ok());

    // Disable, then login is rejected.
    let (st, body) = h
        .patch(
            &format!("/api/v1/admin/users/{carol_id}"),
            &json!({ "status": "disabled" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["status"], "disabled");
    assert_eq!(
        h.login("carol", b"pw-carol").await.unwrap_err(),
        StatusCode::UNAUTHORIZED
    );

    // Re-enable, login works again.
    let (st, _) = h
        .patch(
            &format!("/api/v1/admin/users/{carol_id}"),
            &json!({ "status": "active" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.login("carol", b"pw-carol").await.is_ok());

    // Delete.
    let (st, _) = h
        .delete(&format!("/api/v1/admin/users/{carol_id}"), Some(&admin))
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, users) = h.get("/api/v1/admin/users", Some(&admin)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(users.as_array().unwrap().len(), 1);

    // Deleting a missing user → 404.
    let (st, _) = h
        .delete(&format!("/api/v1/admin/users/{carol_id}"), Some(&admin))
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_changes_roles_and_quotas() {
    let h = Harness::with_config(base_config());
    let admin = h.bootstrap_admin().await;

    let (st, _) = h
        .register("dave", b"pw-dave", Some("dave@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let (_, users) = h.get("/api/v1/admin/users", Some(&admin)).await;
    let dave_id = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "dave")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Promote to admin.
    let (st, body) = h
        .patch(
            &format!("/api/v1/admin/users/{dave_id}"),
            &json!({ "role": "admin" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["role"], "admin");

    // Set a quota override, then clear it (null → default).
    let (st, body) = h
        .patch(
            &format!("/api/v1/admin/users/{dave_id}"),
            &json!({ "quota_bytes": 12345 }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["quota_bytes"], 12345);

    let (st, body) = h
        .patch(
            &format!("/api/v1/admin/users/{dave_id}"),
            &json!({ "quota_bytes": null }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert!(body["quota_bytes"].is_null());

    // Invalid role rejected.
    let (st, _) = h
        .patch(
            &format!("/api/v1/admin/users/{dave_id}"),
            &json!({ "role": "superuser" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

// ── Last-admin protection ────────────────────────────────────────────────────

#[tokio::test]
async fn last_admin_cannot_be_disabled_demoted_or_deleted() {
    let h = Harness::with_config(base_config());
    let admin = h.bootstrap_admin().await;
    let (_, users) = h.get("/api/v1/admin/users", Some(&admin)).await;
    let admin_id = users.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Disable the only admin → forbidden.
    let (st, _) = h
        .patch(
            &format!("/api/v1/admin/users/{admin_id}"),
            &json!({ "status": "disabled" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Demote the only admin → forbidden.
    let (st, _) = h
        .patch(
            &format!("/api/v1/admin/users/{admin_id}"),
            &json!({ "role": "user" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Delete the only admin → forbidden.
    let (st, _) = h
        .delete(&format!("/api/v1/admin/users/{admin_id}"), Some(&admin))
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // With a second admin present, demoting the first is allowed.
    let (salt, verifier) = srp_material("admin2", b"pw-admin2");
    let (st, created) = h
        .post(
            "/api/v1/admin/users",
            &json!({
                "username": "admin2",
                "salt": salt,
                "verifier": verifier,
                "email": "admin2@example.org",
                "role": "admin",
            }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(created["role"], "admin");

    let (st, body) = h
        .patch(
            &format!("/api/v1/admin/users/{admin_id}"),
            &json!({ "role": "user" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["role"], "user");
}

// ── Invites ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_issues_lists_and_revokes_invites_with_end_to_end_redemption() {
    // invite-only so redemption is exercised through the real policy path.
    let cfg = Config {
        registration_policy: RegistrationPolicy::InviteOnly,
        ..base_config()
    };
    let h = Harness::with_config(cfg);
    let admin = h.bootstrap_admin().await; // first user bootstraps as admin

    // Issue an invite.
    let (st, invite) = h
        .post(
            "/api/v1/admin/invites",
            &json!({ "role": "user", "email": "eve@example.org" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let raw_token = invite["token"].as_str().unwrap().to_string();
    let invite_id = invite["id"].as_str().unwrap().to_string();

    // Listing shows it as pending.
    let (st, list) = h.get("/api/v1/admin/invites", Some(&admin)).await;
    assert_eq!(st, StatusCode::OK);
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == invite_id)
        .unwrap();
    assert_eq!(row["status"], "pending");

    // Redeem through the existing register path.
    let (st, body) = h
        .register("eve", b"pw-eve", Some("eve@example.org"), Some(&raw_token))
        .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(body["role"], "user");

    // Now the invite lists as redeemed and can't be reused.
    let (_, list) = h.get("/api/v1/admin/invites", Some(&admin)).await;
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == invite_id)
        .unwrap();
    assert_eq!(row["status"], "redeemed");

    // Issue a second invite, then revoke it (by raw token) → redemption fails.
    let (st, invite2) = h
        .post(
            "/api/v1/admin/invites",
            &json!({ "role": "user" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let raw2 = invite2["token"].as_str().unwrap().to_string();

    let (st, _) = h
        .delete(&format!("/api/v1/admin/invites/{raw2}"), Some(&admin))
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (st, _) = h
        .register("frank", b"pw", Some("frank@example.org"), Some(&raw2))
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "revoked invite must not redeem");

    // Revoking the first invite's id (already redeemed) → 404.
    let (st, _) = h
        .delete(&format!("/api/v1/admin/invites/{invite_id}"), Some(&admin))
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ── Server settings ──────────────────────────────────────────────────────────

#[tokio::test]
async fn settings_are_runtime_updatable_and_consulted_by_registration() {
    // Start Open so registration is initially permissive.
    let h = Harness::with_config(base_config());
    let admin = h.bootstrap_admin().await;

    let (st, s) = h.get("/api/v1/admin/settings", Some(&admin)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(s["registration_policy"], "open");

    // Flip to admin-only at runtime.
    let (st, s) = h
        .patch(
            "/api/v1/admin/settings",
            &json!({ "registration_policy": "admin-only" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(s["registration_policy"], "admin-only");

    // Public registration is now refused (persisted setting beats env Open).
    let (st, _) = h
        .register("gina", b"pw", Some("gina@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Invalid policy rejected.
    let (st, _) = h
        .patch(
            "/api/v1/admin/settings",
            &json!({ "registration_policy": "nonsense" }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn settings_default_quota_is_consulted_by_blob_writes() {
    // Generous env default; we shrink it at runtime so the second blob exceeds.
    let h = Harness::with_config(base_config());
    let admin = h.bootstrap_admin().await;

    // Register + login a regular user who will upload blobs.
    let (st, _) = h
        .register("hugo", b"pw-hugo", Some("hugo@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let token = h.login("hugo", b"pw-hugo").await.expect("login");

    let (st, _) = h
        .post("/api/v1/vaults", &json!({ "vault_id": "v1" }), Some(&token))
        .await;
    assert_eq!(st, StatusCode::CREATED);

    // Tighten the default quota at runtime to 40 bytes.
    let (st, s) = h
        .patch(
            "/api/v1/admin/settings",
            &json!({ "default_quota_bytes": 40 }),
            Some(&admin),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(s["default_quota_bytes"], 40);

    let payload = vec![0u8; 30];
    let (st, _) = h.put_blob("v1", "b1", &payload, &token).await;
    assert_eq!(st, StatusCode::CREATED, "first blob under the new quota");
    let (st, _) = h.put_blob("v1", "b2", &payload, &token).await;
    assert_eq!(
        st,
        StatusCode::PAYLOAD_TOO_LARGE,
        "runtime quota must reject the second blob"
    );
}

// ── Stats ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn stats_reports_user_count_and_per_user_usage() {
    let h = Harness::with_config(base_config());
    let admin = h.bootstrap_admin().await;

    let (st, _) = h
        .register("ivy", b"pw-ivy", Some("ivy@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let token = h.login("ivy", b"pw-ivy").await.expect("login");
    let (st, _) = h
        .post("/api/v1/vaults", &json!({ "vault_id": "v1" }), Some(&token))
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _) = h.put_blob("v1", "b1", &[0u8; 100], &token).await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, stats) = h.get("/api/v1/admin/stats", Some(&admin)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(stats["user_count"], 2);
    assert_eq!(stats["total_storage_bytes"], 100);
    let ivy = stats["per_user"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "ivy")
        .unwrap();
    assert_eq!(ivy["usage_bytes"], 100);
}
