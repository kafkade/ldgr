//! Market data types and provider trait.
//!
//! The provider trait is I/O-free: it builds URLs and parses responses.
//! Platform code (CLI/iOS/web) handles the actual HTTP fetching.

pub mod types;
pub mod yahoo;

pub use types::*;
pub use yahoo::YahooFinance;
