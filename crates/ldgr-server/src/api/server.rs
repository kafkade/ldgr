//! Public, unauthenticated server-discovery endpoints (#177, ADR-008).
//!
//! Before sign-in a client needs to (a) confirm a URL actually points at an
//! ldgr server, (b) learn the wire-protocol version so it can refuse an
//! incompatible server, and (c) read the registration policy and auth
//! capabilities so it can render the right onboarding flow. These endpoints are
//! deliberately unauthenticated and cheap.
//!
//! - `GET /api/v1/server/info` — full discovery document.
//! - `GET /api/v1/server/ping` — tiny liveness probe for URL validation.
//!
//! The container healthcheck keeps using `GET /health` (see [`super::router`]);
//! these endpoints are for *clients*, not orchestrators.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::error::ServerError;
use crate::state::SharedState;
use crate::{MAX_PROTOCOL_VERSION, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION, settings};

/// Discovery document returned by `GET /api/v1/server/info`.
///
/// Designed to grow: new optional capability flags can be added without
/// breaking older clients, which ignore unknown fields. The advertised
/// `registration_policy` is the *effective* one (persisted override resolved
/// against env), so it always matches what `register` enforces.
#[derive(Debug, Serialize)]
pub struct ServerInfo {
    /// Operator-chosen instance label (`LDGR_SERVER_NAME`). Cosmetic.
    pub name: String,
    /// Running server software version (`CARGO_PKG_VERSION`).
    pub version: &'static str,
    /// Current sync/auth wire-protocol version clients compare against.
    pub protocol_version: u32,
    /// Oldest protocol version this server still speaks.
    pub min_protocol_version: u32,
    /// Newest protocol version this server speaks.
    pub max_protocol_version: u32,
    /// Effective registration policy: `open` | `invite-only` | `admin-only`.
    pub registration_policy: &'static str,
    /// Convenience flag: `true` only when anyone may self-register
    /// (`registration_policy == open`).
    pub public_registration: bool,
    /// Whether two-secret (2SKD) auth is available (#172). Always `true`.
    pub two_secret_auth: bool,
}

/// `GET /api/v1/server/info` — unauthenticated discovery document.
pub async fn info(State(state): State<SharedState>) -> Result<Json<ServerInfo>, ServerError> {
    let policy = settings::registration_policy(&state).await?;
    Ok(Json(ServerInfo {
        name: state.config.server_name.clone(),
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: PROTOCOL_VERSION,
        min_protocol_version: MIN_PROTOCOL_VERSION,
        max_protocol_version: MAX_PROTOCOL_VERSION,
        registration_policy: policy.as_str(),
        public_registration: policy == crate::config::RegistrationPolicy::Open,
        two_secret_auth: true,
    }))
}

/// Tiny body for `GET /api/v1/server/ping`.
#[derive(Debug, Serialize)]
pub struct Pong {
    /// Always `true` — lets a client cheaply confirm "this is an ldgr server".
    pub pong: bool,
    /// Server name, echoed so the client can show it during URL validation.
    pub name: String,
    /// Protocol version, so a client can fail fast without a second request.
    pub protocol_version: u32,
}

/// `GET /api/v1/server/ping` — cheap liveness probe (no DB access).
pub async fn ping(State(state): State<SharedState>) -> Json<Pong> {
    Json(Pong {
        pong: true,
        name: state.config.server_name.clone(),
        protocol_version: PROTOCOL_VERSION,
    })
}
