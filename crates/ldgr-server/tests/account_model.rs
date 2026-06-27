//! In-process integration tests for the ADR-008 multi-user account model:
//! schema migration of a legacy (v1) database, first-run admin bootstrap, the
//! three registration policies, disabled-account login rejection, and the
//! aggregate per-user storage quota.
//!
//! Requests are dispatched straight into the axum router via `tower`'s
//! `oneshot` — no sockets bound. The SRP `(salt, verifier)` and login proofs are
//! produced by the real `ldgr-core` client primitives, so the handshake math is
//! exercised end-to-end.

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
    }
}

impl Harness {
    fn with_config(config: Config) -> Self {
        let db = storage::ServerDb::open(":memory:").expect("open in-memory db");
        Self::from_parts(db, config)
    }

    fn from_parts(db: storage::ServerDb, config: Config) -> Self {
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

    async fn post(&self, path: &str, body: &Value, token: Option<&str>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json");
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        let req = builder
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap();
        self.send(req).await
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

    /// Full SRP-6a login handshake through the router; returns the bearer token.
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
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

// ── First-run admin bootstrap ──────────────────────────────────────────────────

#[tokio::test]
async fn first_user_becomes_admin_then_policy_applies() {
    // invite-only with no admin email: first sign-up is the bootstrap admin.
    let cfg = Config {
        registration_policy: RegistrationPolicy::InviteOnly,
        ..base_config()
    };
    let h = Harness::with_config(cfg);

    let (st, body) = h
        .register("alice", b"pw-alice", Some("alice@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED, "first user must be allowed");
    assert_eq!(body["role"], "admin", "first user is bootstrapped as admin");

    // Second sign-up has no invite under invite-only → refused.
    let (st, _) = h
        .register("bob", b"pw-bob", Some("bob@example.org"), None)
        .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "non-first invite-only sign-up needs a token"
    );
}

#[tokio::test]
async fn env_seeded_admin_email_bootstraps_and_bypasses_policy() {
    let cfg = Config {
        registration_policy: RegistrationPolicy::AdminOnly,
        admin_email: Some("admin@example.org".into()),
        ..base_config()
    };
    let h = Harness::with_config(cfg);

    // A non-matching email under admin-only is refused even as the first user.
    let (st, _) = h
        .register("intruder", b"pw", Some("nope@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // The configured admin email registers as admin, bypassing admin-only.
    let (st, body) = h
        .register("root", b"pw-root", Some("admin@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(body["role"], "admin");

    // The seed is one-shot: once the admin email is taken, a second attempt is
    // no longer the bootstrap admin and is refused by the admin-only policy.
    let (st, _) = h
        .register("root2", b"pw", Some("admin@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

// ── Registration policies ───────────────────────────────────────────────────────

#[tokio::test]
async fn open_policy_allows_anyone() {
    let h = Harness::with_config(base_config()); // Open
    let (st, _) = h.register("u1", b"pw", Some("u1@example.org"), None).await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, body) = h.register("u2", b"pw", Some("u2@example.org"), None).await;
    assert_eq!(st, StatusCode::CREATED);
    // Only the first is admin; later open sign-ups are regular users.
    assert_eq!(body["role"], "user");
}

#[tokio::test]
async fn invite_only_redeems_valid_token_and_rejects_reuse() {
    let cfg = Config {
        registration_policy: RegistrationPolicy::InviteOnly,
        admin_email: Some("admin@example.org".into()),
        ..base_config()
    };
    let h = Harness::with_config(cfg);

    // Bootstrap the admin first (env-seeded), so the next user is policy-bound.
    let (st, _) = h
        .register("admin", b"pw-admin", Some("admin@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED);

    // Admin issues an invite (storage seam used directly; admin API is #176).
    let raw_token = "invite-secret-token";
    let token_hash = hex_encode(&sha256(raw_token.as_bytes()));
    h.state
        .db
        .create_invite(&token_hash, None, "user", Some("admin"), None)
        .await
        .unwrap();

    // Wrong token → forbidden.
    let (st, _) = h
        .register("carol", b"pw", Some("carol@example.org"), Some("wrong"))
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Correct token → created.
    let (st, body) = h
        .register("carol", b"pw", Some("carol@example.org"), Some(raw_token))
        .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(body["role"], "user");

    // Token can't be reused.
    let (st, _) = h
        .register("dave", b"pw", Some("dave@example.org"), Some(raw_token))
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_only_refuses_public_registration() {
    let cfg = Config {
        registration_policy: RegistrationPolicy::AdminOnly,
        admin_email: Some("admin@example.org".into()),
        ..base_config()
    };
    let h = Harness::with_config(cfg);
    // Bootstrap admin via env seed.
    let (st, _) = h
        .register("admin", b"pw", Some("admin@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED);
    // Everyone else is refused on the public endpoint.
    let (st, _) = h
        .register("eve", b"pw", Some("eve@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

// ── Disabled login ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn disabled_account_cannot_log_in() {
    let h = Harness::with_config(base_config()); // Open
    let (st, _) = h
        .register("frank", b"pw-frank", Some("frank@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED);

    // Active account logs in fine.
    assert!(h.login("frank", b"pw-frank").await.is_ok());

    // Disable, then login must be Unauthorized.
    assert!(
        h.state
            .db
            .set_user_status("frank", "disabled")
            .await
            .unwrap()
    );
    let err = h
        .login("frank", b"pw-frank")
        .await
        .expect_err("disabled login");
    assert_eq!(err, StatusCode::UNAUTHORIZED);
}

// ── Quota enforcement ───────────────────────────────────────────────────────────

#[tokio::test]
async fn aggregate_quota_is_enforced_on_blob_writes() {
    // 40-byte quota; blobs are 30 bytes each so the second exceeds the cap.
    let cfg = Config {
        default_user_quota_bytes: 40,
        ..base_config()
    };
    let h = Harness::with_config(cfg);
    let (st, _) = h
        .register("gina", b"pw-gina", Some("gina@example.org"), None)
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let token = h.login("gina", b"pw-gina").await.expect("login");

    // Create a vault via the API.
    let (st, _) = h
        .post(
            "/api/v1/vaults",
            &json!({ "vault_id": "vault-1" }),
            Some(&token),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);

    let payload = vec![0u8; 30];
    // First blob (30 ≤ 40) succeeds.
    let (st, _) = h.put_blob("vault-1", "dev", "b1", &payload, &token).await;
    assert_eq!(st, StatusCode::CREATED, "first blob is under quota");

    // Second blob (30 + 30 = 60 > 40) is rejected with 413.
    let (st, _) = h.put_blob("vault-1", "dev", "b2", &payload, &token).await;
    assert_eq!(
        st,
        StatusCode::PAYLOAD_TOO_LARGE,
        "aggregate quota must reject the second blob"
    );
}

impl Harness {
    async fn put_blob(
        &self,
        vault: &str,
        device: &str,
        batch: &str,
        body: &[u8],
        token: &str,
    ) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/vaults/{vault}/batches/{device}/{batch}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_vec()))
            .unwrap();
        self.send(req).await
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).into()
}

// ── Migration of a legacy (v1) database ─────────────────────────────────────────

#[tokio::test]
async fn migration_upgrades_legacy_db_with_defaults_and_preserves_auth() {
    // 1. Hand-build a pre-ADR-008 database (the original 5-column users table)
    //    with one legacy account, using a real SRP verifier.
    let dir = std::env::temp_dir();
    let path = dir.join(format!("ldgr-migrate-{}.db", uuid::Uuid::now_v7()));
    let path_str = path.to_str().unwrap().to_string();

    let reg = register_with_salt("legacy", b"legacy-pw", vec![0x5a; 16]);
    {
        let conn = rusqlite::Connection::open(&path_str).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                 id          TEXT PRIMARY KEY,
                 username    TEXT UNIQUE NOT NULL,
                 salt        BLOB NOT NULL,
                 verifier    BLOB NOT NULL,
                 created_at  TEXT NOT NULL
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (id, username, salt, verifier, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "legacy-id",
                "legacy",
                reg.salt,
                reg.verifier,
                "2020-01-01T00:00:00Z"
            ],
        )
        .unwrap();
    }

    // 2. Opening through ServerDb runs the additive migration.
    let db = storage::ServerDb::open(&path_str).expect("open + migrate");

    // 3. The legacy row is backfilled with the documented defaults.
    let user = db
        .get_user_by_username("legacy")
        .await
        .unwrap()
        .expect("legacy user survives migration");
    assert_eq!(user.role, "user");
    assert_eq!(user.status, "active");

    // auth_scheme defaults to the legacy single-secret scheme.
    {
        let conn = rusqlite::Connection::open(&path_str).unwrap();
        let scheme: String = conn
            .query_row(
                "SELECT auth_scheme FROM users WHERE username = 'legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(scheme, "srp-1secret");
        // The new columns/index exist and email is NULL for the legacy row.
        let email: Option<String> = conn
            .query_row(
                "SELECT email FROM users WHERE username = 'legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(email.is_none());
    }

    // 4. The legacy account still authenticates through the real handshake.
    let h = Harness::from_parts(db, base_config());
    let token = h.login("legacy", b"legacy-pw").await.expect("legacy login");
    assert!(!token.is_empty());

    // 5. Re-opening the already-migrated DB is idempotent (no errors).
    drop(h);
    let _ = storage::ServerDb::open(&path_str).expect("re-open is idempotent");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{path_str}-wal"));
    let _ = std::fs::remove_file(format!("{path_str}-shm"));
}
