//! `SQLite` storage layer for ldgr.
//!
//! The vault is internally a `SQLite` database with versioned rows (soft deletes,
//! version column). All decimal amounts are stored as TEXT for precision.
//!
//! This module is gated behind the `sqlite` feature flag.

#[cfg(feature = "sqlite")]
pub mod accounts;
#[cfg(feature = "sqlite")]
pub mod error;
#[cfg(all(feature = "sqlite", feature = "goals"))]
pub mod goals;
#[cfg(feature = "sqlite")]
pub mod schema;
#[cfg(feature = "sqlite")]
pub mod sync;
#[cfg(feature = "sqlite")]
pub mod transactions;
