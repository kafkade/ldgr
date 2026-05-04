//! `SQLite` storage layer for ldgr.
//!
//! The vault is internally a `SQLite` database with versioned rows (soft deletes,
//! version column). All decimal amounts are stored as TEXT for precision.

// SQLite-backed implementations require the `sqlite` feature.
// #[cfg(feature = "sqlite")]
// pub mod schema;    // Table definitions and migrations
// #[cfg(feature = "sqlite")]
// pub mod accounts;  // Account CRUD
// #[cfg(feature = "sqlite")]
// pub mod transactions; // Transaction + posting CRUD
// pub mod queries;   // Query engine (platform-independent)
