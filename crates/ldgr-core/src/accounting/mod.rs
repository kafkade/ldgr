//! Accounting engine: hledger-compatible journal parser and core types.
//!
//! This module implements a strict subset of hledger's journal format.
//! See `docs/journal-subset.md` for the full specification.

pub mod lots;
pub mod parser;
pub mod query;
pub mod report_document;
pub mod reports;
pub mod types;

pub use lots::{
    CostBasisMethod, DisposalResult, GainEntry, HoldingTerm, Lot, LotConsumption, classify_term,
    dispose_average, dispose_fifo, dispose_lifo, dispose_specific, unrealized_gain,
};
pub use parser::{ParseError, parse_journal};
pub use query::{Filter, Query};
pub use report_document::{
    Amount, ReportDocument, ReportRow, ReportSection, balance_sheet_document,
    income_statement_document, net_worth_document,
};
pub use reports::{
    AccountBalance, BalanceReport, BalanceSheet, CashFlow, IncomeStatement, NetWorth,
    RegisterEntry, RegisterReport, TrialBalance, TrialBalanceEntry, compute_balance,
    compute_balance_sheet, compute_balance_with_query, compute_cash_flow, compute_income_statement,
    compute_net_worth, compute_register, compute_register_with_query, compute_trial_balance,
};
pub use types::*;
