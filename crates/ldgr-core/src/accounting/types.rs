//! Core accounting types for the hledger-compatible journal subset.

use std::collections::HashMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A parsed journal containing transactions and declarations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Journal {
    pub transactions: Vec<Transaction>,
    pub account_declarations: Vec<AccountDeclaration>,
    pub commodity_declarations: Vec<CommodityDeclaration>,
    pub price_directives: Vec<PriceDirective>,
}

/// Transaction status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Unmarked,
    Pending,
    Cleared,
}

/// A parsed transaction with its postings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub date: String,
    pub status: Status,
    pub code: Option<String>,
    pub description: String,
    pub postings: Vec<Posting>,
    pub tags: HashMap<String, String>,
    pub comment: Option<String>,
    /// 1-based line number in the source journal.
    pub source_line: usize,
}

/// A posting within a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Posting {
    pub account: String,
    pub amount: Option<Amount>,
    pub balance_assertion: Option<Amount>,
    pub status: Status,
    pub comment: Option<String>,
    pub tags: HashMap<String, String>,
}

/// A monetary amount with quantity and optional commodity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Amount {
    pub quantity: Decimal,
    pub commodity: String,
}

/// An account declaration (`account Assets:Checking`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountDeclaration {
    pub name: String,
    pub source_line: usize,
}

/// A commodity declaration (`commodity USD`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommodityDeclaration {
    pub symbol: String,
    pub source_line: usize,
}

/// A price directive (`P 2024-01-15 AAPL 185.50 USD`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceDirective {
    pub date: String,
    pub commodity: String,
    pub price: Amount,
    pub source_line: usize,
}
