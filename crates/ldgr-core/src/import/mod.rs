//! Import pipeline: CSV/OFX parsing, column mapping, deduplication, and auto-categorization.
//!
//! This module is pure computation — no I/O. Callers provide raw text
//! and receive structured import candidates.

pub mod csv;
pub mod dedup;
pub mod ofx;
pub mod profile;
pub mod rules;

pub use csv::parse_csv;
pub use dedup::{DedupResult, ExistingTransaction, check_duplicate, deduplicate};
pub use ofx::parse_ofx;
pub use profile::CsvProfile;
pub use rules::{ImportRule, MatchType, apply_rules};
