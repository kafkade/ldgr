//! Loan tracking module: amortization, payoff projections, refinance comparison,
//! and payment auto-split.
//!
//! Pure computation — no I/O, no networking. All monetary values use
//! `rust_decimal::Decimal`. Interest rates are annual decimal fractions
//! (e.g. 6.5% = `Decimal::new(65, 3)` = `0.065`).

pub mod amortization;
pub mod payoff;
pub mod refinance;
pub mod split;

pub use amortization::*;
pub use payoff::*;
pub use refinance::*;
pub use split::*;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Loan type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoanType {
    Mortgage,
    Auto,
    Student,
    Personal,
    Heloc,
}

/// Interest rate type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RateType {
    Fixed,
    /// Variable rate that adjusts at a regular interval.
    Variable {
        /// How often the rate adjusts, in months.
        adjust_period_months: u32,
    },
}

/// A loan definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Loan {
    pub id: String,
    pub name: String,
    pub loan_type: LoanType,
    /// Original principal balance.
    pub principal: Decimal,
    /// Annual interest rate as a decimal fraction (6.5% = 0.065).
    pub annual_rate: Decimal,
    pub rate_type: RateType,
    /// Loan term in months.
    pub term_months: u32,
    pub start_date: NaiveDate,
    /// Monthly payment amount (principal + interest, excluding extra).
    pub payment_amount: Decimal,
    /// Extra monthly principal payment.
    pub extra_payment: Decimal,
    /// Linked liability account in the ledger.
    pub linked_account: String,
}

/// A single row in an amortization schedule.
#[derive(Debug, Clone)]
pub struct AmortizationEntry {
    /// 1-based month number.
    pub month: u32,
    pub date: NaiveDate,
    /// Total payment applied this month (principal + interest + extra).
    pub payment: Decimal,
    /// Principal portion of the regular payment.
    pub principal: Decimal,
    /// Interest portion of the regular payment.
    pub interest: Decimal,
    /// Extra principal payment applied.
    pub extra_payment: Decimal,
    /// Remaining balance after this payment.
    pub balance: Decimal,
}

/// A complete amortization schedule.
#[derive(Debug, Clone)]
pub struct AmortizationSchedule {
    pub entries: Vec<AmortizationEntry>,
    /// Total interest paid over the life of the loan.
    pub total_interest: Decimal,
    /// Total amount paid (principal + interest + extra).
    pub total_paid: Decimal,
    /// Date of the final payment.
    pub payoff_date: NaiveDate,
    /// Number of months to payoff.
    pub months_to_payoff: u32,
}

/// A rate adjustment for variable-rate schedule projections.
#[derive(Debug, Clone)]
pub struct RateAdjustment {
    /// Month number (1-based) when this rate takes effect.
    pub effective_month: u32,
    /// New annual interest rate as a decimal fraction.
    pub annual_rate: Decimal,
}

/// Options for schedule generation.
#[derive(Debug, Clone, Default)]
pub struct ScheduleOptions {
    /// Rate adjustments for variable-rate loans, sorted by `effective_month`.
    pub rate_adjustments: Vec<RateAdjustment>,
    /// Override the extra monthly payment (uses `loan.extra_payment` if `None`).
    pub extra_payment_override: Option<Decimal>,
}

/// Errors from loan computations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum LoanError {
    #[error("term_months must be greater than 0")]
    ZeroTerm,
    #[error("principal must be positive")]
    NonPositivePrincipal,
    #[error("annual_rate must be non-negative")]
    NegativeRate,
    #[error("payment_amount must be positive")]
    NonPositivePayment,
    #[error("payment does not cover monthly interest ({interest}); negative amortization")]
    NegativeAmortization { interest: Decimal },
    #[error("rate adjustments must be sorted by effective_month")]
    UnsortedAdjustments,
}

/// Two-decimal-place rounding for currency values.
pub(crate) fn round_currency(d: Decimal) -> Decimal {
    d.round_dp(2)
}

/// Compute (1 + r)^n using iterative multiplication for `Decimal`.
pub(crate) fn decimal_pow(base: Decimal, exp: u32) -> Decimal {
    let mut result = Decimal::ONE;
    let mut b = base;
    let mut e = exp;
    // Square-and-multiply for efficiency
    while e > 0 {
        if e % 2 == 1 {
            result *= b;
        }
        b *= b;
        e /= 2;
    }
    result
}
