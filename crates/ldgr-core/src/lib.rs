//! ldgr-core: zero-knowledge bookkeeping core library.
//!
//! This crate contains all computation logic — crypto, accounting, storage, sync,
//! import/export, market data, loans, budgeting, and goals.
//!
//! **No I/O, no networking, no platform APIs.** All I/O happens in platform-specific
//! code (CLI, iOS, web). This keeps the core testable and compilable to WASM.

pub mod accounting;
pub mod crypto;
pub mod import;
pub mod storage;

// Feature-gated modules (uncomment as implemented)
// pub mod sync;
// pub mod import;
// pub mod export;
// pub mod market;
// pub mod loans;
// pub mod budget;
// pub mod goals;
