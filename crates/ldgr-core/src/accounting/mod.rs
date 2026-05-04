//! Accounting engine: hledger-compatible journal parser and core types.
//!
//! This module implements a strict subset of hledger's journal format.
//! See `docs/journal-subset.md` for the full specification.

pub mod parser;
pub mod reports;
pub mod types;

pub use parser::{ParseError, parse_journal};
pub use reports::{
    AccountBalance, BalanceReport, RegisterEntry, RegisterReport, compute_balance, compute_register,
};
pub use types::*;
