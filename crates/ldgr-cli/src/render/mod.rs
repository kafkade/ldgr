//! Output rendering that lives in the CLI (I/O-capable) layer.
//!
//! `ldgr-core` produces layout-agnostic [`ReportDocument`]s; this module turns
//! them into concrete output bytes (currently PDF). Kept out of core so heavier
//! rendering dependencies never reach the WASM bundle (ADR-005).

pub mod pdf;
