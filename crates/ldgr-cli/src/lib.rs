//! Library surface of the `ldgr` CLI.
//!
//! The CLI is primarily a binary (`src/main.rs`), but a small library target is
//! exposed so integration tests can exercise reusable pieces in-process. This
//! includes the sync transport layer (`sync`), whose `ServerTransport` is
//! covered by `tests/server_sync.rs` against a live in-process `ldgr-server`,
//! and the market-data fetch pipeline (`market_fetch`).
//!
//! The binary keeps its own module tree; this surface intentionally re-exports
//! only the self-contained, dependency-light modules worth testing directly.

pub mod market_fetch;
pub mod sync;
