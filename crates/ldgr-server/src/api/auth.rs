//! Auth API endpoints: register, login (SRP-6a two-step).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

use crate::auth::middleware::AuthUser;
use crate::auth::session::generate_token;
use crate::auth::{hex_decode, hex_encode};
use crate::error::ServerError;
use crate::state::SharedState;

// ── Register ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub salt: String,     // hex-encoded
    pub verifier: String, // hex-encoded
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub user_id: String,
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

    let user_id = uuid::Uuid::now_v7().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    state
        .db
        .create_user(&user_id, &req.username, &salt, &verifier, &created_at)
        .await?;

    tracing::info!("registered user: {}", req.username);

    Ok((StatusCode::CREATED, Json(RegisterResponse { user_id })))
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
