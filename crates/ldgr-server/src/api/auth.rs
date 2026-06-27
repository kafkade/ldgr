//! Auth API endpoints: register, login (SRP-6a two-step).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth::middleware::AuthUser;
use crate::auth::session::generate_token;
use crate::auth::{hex_decode, hex_encode};
use crate::config::RegistrationPolicy;
use crate::error::ServerError;
use crate::state::SharedState;
use crate::storage::NewUser;

// ── Register ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub salt: String,     // hex-encoded
    pub verifier: String, // hex-encoded
    /// Canonical sign-in email (ADR-008 Decision 5). Optional on the wire for
    /// backward compatibility with username-only clients; defaults to
    /// `username` (which clients use as the email per ADR-008).
    #[serde(default)]
    pub email: Option<String>,
    /// Invite token, required under the `invite-only` registration policy
    /// (ignored for the bootstrap admin and under `open`).
    #[serde(default)]
    pub invite_token: Option<String>,
    /// SRP derivation scheme: `srp-1secret` (legacy, default) or `srp-2skd-v1`.
    /// The wire handshake is identical; this only records how the client
    /// derived `x` so legacy accounts stay distinguishable (ADR-008 Migration).
    #[serde(default)]
    pub auth_scheme: Option<String>,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub user_id: String,
    pub role: String,
}

/// Hash an opaque token (invite/session-style) for at-rest storage.
fn hash_token(token: &str) -> String {
    hex_encode(&Sha256::digest(token.as_bytes()))
}

pub async fn register(
    State(state): State<SharedState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), ServerError> {
    if req.username.is_empty() || req.username.len() > 128 {
        return Err(ServerError::BadRequest(
            "username must be 1-128 characters".into(),
        ));
    }

    let salt =
        hex_decode(&req.salt).map_err(|e| ServerError::BadRequest(format!("invalid salt: {e}")))?;
    if salt.len() < 16 {
        return Err(ServerError::BadRequest(
            "salt must be at least 16 bytes".into(),
        ));
    }

    let verifier = hex_decode(&req.verifier)
        .map_err(|e| ServerError::BadRequest(format!("invalid verifier: {e}")))?;
    if verifier.is_empty() {
        return Err(ServerError::BadRequest("verifier must not be empty".into()));
    }

    // ADR-008: email is the canonical sign-in identity. Username-only clients
    // use the username as the email, so default to it when omitted.
    let email = req
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(req.username.as_str())
        .to_string();

    // Validate the recorded auth scheme (the wire handshake is unchanged).
    let auth_scheme = req.auth_scheme.as_deref().unwrap_or("srp-1secret");
    if !matches!(auth_scheme, "srp-1secret" | "srp-2skd-v1") {
        return Err(ServerError::BadRequest(format!(
            "unsupported auth_scheme: {auth_scheme}"
        )));
    }

    let user_id = uuid::Uuid::now_v7().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    // ── First-run admin bootstrap (ADR-008 Decision 5) ──────────────────────
    //
    // Two supported modes:
    //   * Env-seeded (preferred for unattended docker-compose): if
    //     `LDGR_ADMIN_EMAIL` is set, the account registering with that email
    //     becomes admin and bypasses the registration policy.
    //   * First-user fallback: if no admin email is configured, the very first
    //     account to register (empty user table) becomes admin and bypasses the
    //     policy. After that, the policy governs all sign-ups.
    let is_bootstrap_admin = if let Some(admin_email) = state.config.admin_email.as_deref() {
        email.eq_ignore_ascii_case(admin_email) && !state.db.email_exists(admin_email).await?
    } else {
        state.db.count_users().await? == 0
    };

    // ── Registration policy enforcement ─────────────────────────────────────
    let mut role = if is_bootstrap_admin { "admin" } else { "user" }.to_string();
    let mut invited_by: Option<String> = None;
    if !is_bootstrap_admin {
        match crate::settings::registration_policy(&state).await? {
            RegistrationPolicy::Open => {}
            RegistrationPolicy::InviteOnly => {
                let token = req
                    .invite_token
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .ok_or(ServerError::Forbidden("registration is invite-only".into()))?;
                let grant = state
                    .db
                    .redeem_invite(&hash_token(token), &user_id)
                    .await?
                    .ok_or(ServerError::Forbidden(
                        "invalid or already-used invite token".into(),
                    ))?;
                // An invite may pin the granted role and the bound email.
                if grant
                    .email
                    .as_deref()
                    .is_some_and(|bound| !email.eq_ignore_ascii_case(bound))
                {
                    return Err(ServerError::Forbidden(
                        "invite was issued for a different email".into(),
                    ));
                }
                if grant.role == "admin" {
                    role = "admin".to_string();
                }
                invited_by = grant.created_by;
            }
            RegistrationPolicy::AdminOnly => {
                // Public self-registration is refused; account creation is an
                // admin action (admin API lands in #176).
                return Err(ServerError::Forbidden(
                    "registration is restricted to administrators".into(),
                ));
            }
        }
    }

    state
        .db
        .create_user(&NewUser {
            id: &user_id,
            username: &req.username,
            email: Some(&email),
            salt: &salt,
            verifier: &verifier,
            role: &role,
            auth_scheme,
            invited_by: invited_by.as_deref(),
            created_at: &created_at,
        })
        .await?;

    tracing::info!("registered user: {} (role={role})", req.username);

    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse { user_id, role }),
    ))
}

// ── Login Init (SRP Step 1) ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LoginInitRequest {
    pub username: String,
    pub client_public: String, // hex-encoded A
}

#[derive(Serialize)]
pub struct LoginInitResponse {
    pub handshake_id: String,
    pub salt: String,          // hex-encoded
    pub server_public: String, // hex-encoded B
}

pub async fn login_init(
    State(state): State<SharedState>,
    Json(req): Json<LoginInitRequest>,
) -> Result<Json<LoginInitResponse>, ServerError> {
    let user = state
        .db
        .get_user_by_username(&req.username)
        .await?
        .ok_or(ServerError::Unauthorized)?;

    // Disabled accounts cannot sign in (ADR-008 account status).
    if user.status == "disabled" {
        return Err(ServerError::Unauthorized);
    }

    let a_bytes = hex_decode(&req.client_public)
        .map_err(|e| ServerError::BadRequest(format!("invalid client_public: {e}")))?;
    let a = BigUint::from_bytes_be(&a_bytes);

    let verifier = BigUint::from_bytes_be(&user.verifier);
    let handshake_id = uuid::Uuid::now_v7().to_string();

    let b_pub = state
        .srp_handshakes
        .initiate(
            handshake_id.clone(),
            user.username,
            a,
            user.salt.clone(),
            verifier,
        )
        .map_err(ServerError::BadRequest)?;

    Ok(Json(LoginInitResponse {
        handshake_id,
        salt: hex_encode(&user.salt),
        server_public: hex_encode(&b_pub.to_bytes_be()),
    }))
}

// ── Login Verify (SRP Step 2) ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LoginVerifyRequest {
    pub handshake_id: String,
    pub client_proof: String, // hex-encoded M1
}

#[derive(Serialize)]
pub struct LoginVerifyResponse {
    pub server_proof: String, // hex-encoded M2
    pub token: String,
}

pub async fn login_verify(
    State(state): State<SharedState>,
    Json(req): Json<LoginVerifyRequest>,
) -> Result<Json<LoginVerifyResponse>, ServerError> {
    let m1_bytes = hex_decode(&req.client_proof)
        .map_err(|e| ServerError::BadRequest(format!("invalid client_proof: {e}")))?;

    let (m2, username) = state
        .srp_handshakes
        .verify(&req.handshake_id, &m1_bytes)
        .map_err(|_| ServerError::Unauthorized)?;

    // Look up user for session creation
    let user = state
        .db
        .get_user_by_username(&username)
        .await?
        .ok_or(ServerError::Unauthorized)?;

    // Reject sessions for accounts disabled mid-handshake.
    if user.status == "disabled" {
        return Err(ServerError::Unauthorized);
    }

    // Create session
    let (raw_token, token_hash) = generate_token();
    let now = chrono::Utc::now();
    #[allow(clippy::cast_possible_wrap)]
    let expires = now + chrono::Duration::hours(state.config.session_ttl_hours as i64);

    state
        .db
        .create_session(
            &token_hash,
            &user.id,
            &now.to_rfc3339(),
            &expires.to_rfc3339(),
        )
        .await?;

    tracing::info!("user logged in: {username}");

    Ok(Json(LoginVerifyResponse {
        server_proof: hex_encode(&m2),
        token: raw_token,
    }))
}

// ── Logout ────────────────────────────────────────────────────────────────────

pub async fn logout(State(_state): State<SharedState>, AuthUser(_user_id): AuthUser) -> StatusCode {
    // Session is already validated by the AuthUser extractor.
    // In a full implementation we'd delete the session from the DB.
    StatusCode::NO_CONTENT
}
