//! Budgeting module: envelope and zero-based budgets.
//!
//! Pure computation — budget definitions and vs-actual comparison.

pub mod engine;
pub mod recurring;

pub use engine::*;
pub use recurring::*;
