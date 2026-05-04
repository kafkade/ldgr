//! Accounting engine: hledger-compatible journal parser and core types.
//!
//! This module implements a strict subset of hledger's journal format.
//! See `docs/journal-subset.md` for the full specification.

pub mod parser;
pub mod query;
pub mod reports;
pub mod types;

pub use parser::{ParseError, parse_journal};
pub use query::{Filter, Query};
pub use reports::{
    AccountBalance, BalanceReport, BalanceSheet, IncomeStatement, RegisterEntry, RegisterReport,
    compute_balance, compute_balance_sheet, compute_balance_with_query, compute_income_statement,
    compute_register, compute_register_with_query,
};
pub use types::*;
