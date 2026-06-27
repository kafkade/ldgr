//! ldgr-server library: the encrypted blob sync server's building blocks.
//!
//! Exposed as a library so integration tests (and embedders) can construct the
//! application [`router`](api::router) and drive it in-process without binding a
//! socket. The `main.rs` binary is a thin wrapper around these modules.
//!
//! AGPL-3.0-only — see this crate's `LICENSE`.

pub mod api;
pub mod auth;
pub mod config;
pub mod error;
pub mod settings;
pub mod state;
pub mod storage;
