//! Payoff projections: extra payments, biweekly scenarios.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::amortization::generate_schedule;
use super::{Loan, LoanError, ScheduleOptions, round_currency};

/// Result of a payoff projection compared to the baseline schedule.
#[derive(Debug, Clone)]
pub struct PayoffProjection {
    /// Monthly extra payment applied.
    pub extra_monthly: Decimal,
    /// Number of months to payoff with extra payments.
    pub months_to_payoff: u32,
    /// Months saved compared to baseline.
    pub months_saved: u32,
    /// Total interest paid.
    pub total_interest: Decimal,
    /// Interest saved compared to baseline.
    pub interest_saved: Decimal,
    /// Total amount paid.
    pub total_paid: Decimal,
    /// Final payoff date.
    pub payoff_date: NaiveDate,
}

/// Project the effect of additional monthly payments.
///
/// Compares the loan with and without the given extra monthly payment.
pub fn project_extra_payments(
    loan: &Loan,
    extra_monthly: Decimal,
) -> Result<PayoffProjection, LoanError> {
    // Baseline schedule (no extra payments beyond what's in the loan)
    let baseline_opts = ScheduleOptions {
        extra_payment_override: Some(Decimal::ZERO),
        ..ScheduleOptions::default()
    };
    let baseline = generate_schedule(loan, &baseline_opts)?;

    // Schedule with extra payments
    let extra_opts = ScheduleOptions {
        extra_payment_override: Some(loan.extra_payment + extra_monthly),
        ..ScheduleOptions::default()
    };
    let with_extra = generate_schedule(loan, &extra_opts)?;

    Ok(PayoffProjection {
        extra_monthly: loan.extra_payment + extra_monthly,
        months_to_payoff: with_extra.months_to_payoff,
        months_saved: baseline
            .months_to_payoff
            .saturating_sub(with_extra.months_to_payoff),
        total_interest: with_extra.total_interest,
        interest_saved: round_currency(baseline.total_interest - with_extra.total_interest),
        total_paid: with_extra.total_paid,
        payoff_date: with_extra.payoff_date,
    })
}

/// Project the effect of biweekly payments.
///
/// Biweekly = half the monthly payment every 2 weeks = 26 half-payments/year
/// = 13 monthly-equivalent payments per year (one extra payment per year).
pub fn project_biweekly(loan: &Loan) -> Result<PayoffProjection, LoanError> {
    // One extra monthly payment per year, spread monthly = payment / 12
    let biweekly_extra = round_currency(loan.payment_amount / Decimal::from(12));
    project_extra_payments(loan, biweekly_extra)
}

/// Compare multiple extra payment scenarios side by side.
pub fn compare_scenarios(
    loan: &Loan,
    extras: &[Decimal],
) -> Result<Vec<PayoffProjection>, LoanError> {
    extras
        .iter()
        .map(|&extra| project_extra_payments(loan, extra))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loans::amortization::compute_monthly_payment;
    use crate::loans::{LoanType, RateType};

    fn test_mortgage() -> Loan {
        let payment =
            compute_monthly_payment(Decimal::new(200_000, 0), Decimal::new(65, 3), 360).unwrap();
        Loan {
            id: "m1".into(),
            name: "Mortgage".into(),
            loan_type: LoanType::Mortgage,
            principal: Decimal::new(200_000, 0),
            annual_rate: Decimal::new(65, 3),
            rate_type: RateType::Fixed,
            term_months: 360,
            start_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            payment_amount: payment,
            extra_payment: Decimal::ZERO,
            linked_account: "Liabilities:Mortgage".into(),
        }
    }

    #[test]
    fn extra_200_per_month() {
        let loan = test_mortgage();
        let projection = project_extra_payments(&loan, Decimal::new(200, 0)).unwrap();

        // Should save significant time
        assert!(projection.months_saved > 60); // > 5 years saved
        assert!(projection.interest_saved > Decimal::new(50_000, 0));
        assert!(projection.months_to_payoff < 300);
    }

    #[test]
    fn biweekly_saves_time() {
        let loan = test_mortgage();
        let projection = project_biweekly(&loan).unwrap();

        assert!(projection.months_saved > 0);
        assert!(projection.interest_saved > Decimal::ZERO);
    }

    #[test]
    fn compare_multiple_scenarios() {
        let loan = test_mortgage();
        let extras = vec![
            Decimal::new(100, 0),
            Decimal::new(200, 0),
            Decimal::new(500, 0),
        ];
        let results = compare_scenarios(&loan, &extras).unwrap();

        assert_eq!(results.len(), 3);
        // More extra = fewer months
        assert!(results[0].months_to_payoff > results[1].months_to_payoff);
        assert!(results[1].months_to_payoff > results[2].months_to_payoff);
        // More extra = more interest saved
        assert!(results[0].interest_saved < results[1].interest_saved);
        assert!(results[1].interest_saved < results[2].interest_saved);
    }

    #[test]
    fn zero_extra_matches_baseline() {
        let loan = test_mortgage();
        let projection = project_extra_payments(&loan, Decimal::ZERO).unwrap();

        assert_eq!(projection.months_saved, 0);
        assert_eq!(projection.interest_saved, Decimal::ZERO);
        assert_eq!(projection.months_to_payoff, 360);
    }
}
