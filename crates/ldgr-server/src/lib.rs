//! ldgr-server library: the encrypted blob sync server's building blocks.
//!
//! Exposed as a library so integration tests (and embedders) can construct the
//! application [`router`](api::router) and drive it in-process without binding a
//! socket. The `main.rs` binary is a thin wrapper around these modules.
//!
//! AGPL-3.0-only — see this crate's `LICENSE`.

/// Sync/auth wire-protocol version advertised by `GET /api/v1/server/info`.
///
/// This is the single source of truth clients compare against to decide whether
/// they can talk to this server. Bump it whenever a **breaking** change is made
/// to the sync or auth protocol (request/response shapes, handshake flow, blob
/// framing, etc.). The client side (`ldgr-core`'s `sync::server`) is expected to
/// refuse a server whose protocol falls outside the range it supports — but that
/// client-side enforcement lives in the transport work (#162/#163/#165); this
/// crate only has to *expose* the version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Oldest protocol version this server still speaks. Equal to
/// [`PROTOCOL_VERSION`] until we need to maintain backwards compatibility across
/// a breaking bump, at which point this advertises the supported range's floor.
pub const MIN_PROTOCOL_VERSION: u32 = 1;

/// Newest protocol version this server speaks (currently the only one).
pub const MAX_PROTOCOL_VERSION: u32 = PROTOCOL_VERSION;

pub mod api;
pub mod auth;
pub mod config;
pub mod error;
pub mod settings;
pub mod state;
pub mod storage;
