//! Import pipeline: CSV parsing, column mapping, and auto-categorization.
//!
//! This module is pure computation — no I/O. Callers provide raw text
//! and receive structured import candidates.

pub mod csv;
pub mod profile;
pub mod rules;

pub use csv::parse_csv;
pub use profile::CsvProfile;
pub use rules::{ImportRule, MatchType, apply_rules};
