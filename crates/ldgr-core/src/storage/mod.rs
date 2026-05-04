//! `SQLite` storage layer for ldgr.
//!
//! The vault is internally a `SQLite` database with versioned rows (soft deletes,
//! version column). All decimal amounts are stored as TEXT for precision.

// pub mod schema;    // Table definitions and migrations
// pub mod accounts;  // Account CRUD
// pub mod transactions; // Transaction + posting CRUD
// pub mod queries;   // Query engine
