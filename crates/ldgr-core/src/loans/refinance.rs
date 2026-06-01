//! Refinance comparison: simulate old vs new loan, find break-even month.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::amortization::{compute_monthly_payment, generate_schedule};
use super::{Loan, LoanError, LoanType, RateType, ScheduleOptions, round_currency};

/// Result of a refinance comparison.
#[derive(Debug, Clone)]
pub struct RefinanceComparison {
    /// New monthly payment.
    pub new_payment: Decimal,
    /// Monthly savings (old payment - new payment).
    pub monthly_savings: Decimal,
    /// New loan total interest.
    pub new_total_interest: Decimal,
    /// Current loan remaining interest.
    pub current_remaining_interest: Decimal,
    /// Net savings (old remaining cost - new total cost including closing costs).
    /// Negative means refinancing costs more.
    pub net_savings: Decimal,
    /// Closing costs.
    pub closing_costs: Decimal,
    /// Break-even month: after this many months, refinancing saves money.
    /// `None` if refinancing never breaks even.
    pub break_even_month: Option<u32>,
    /// New payoff date.
    pub new_payoff_date: NaiveDate,
    /// New term in months.
    pub new_term_months: u32,
}

/// Compare refinancing the current loan vs keeping it.
///
/// - `remaining_balance`: current outstanding principal
/// - `months_remaining`: months left on the current loan
/// - `current_rate`: current annual interest rate
/// - `new_rate`: proposed new annual rate
/// - `new_term_months`: term of the new loan
/// - `closing_costs`: one-time refinance costs
/// - `start_date`: when the new loan would start
#[allow(clippy::too_many_arguments)]
pub fn compare_refinance(
    remaining_balance: Decimal,
    months_remaining: u32,
    current_rate: Decimal,
    new_rate: Decimal,
    new_term_months: u32,
    closing_costs: Decimal,
    start_date: NaiveDate,
) -> Result<RefinanceComparison, LoanError> {
    if remaining_balance <= Decimal::ZERO {
        return Err(LoanError::NonPositivePrincipal);
    }
    if months_remaining == 0 || new_term_months == 0 {
        return Err(LoanError::ZeroTerm);
    }
    if new_rate < Decimal::ZERO {
        return Err(LoanError::NegativeRate);
    }

    let current_payment =
        compute_monthly_payment(remaining_balance, current_rate, months_remaining)?;
    let new_payment = compute_monthly_payment(remaining_balance, new_rate, new_term_months)?;

    // Generate both schedules for month-by-month comparison
    let current_loan = Loan {
        id: "current".into(),
        name: "Current".into(),
        loan_type: LoanType::Personal,
        principal: remaining_balance,
        annual_rate: current_rate,
        rate_type: RateType::Fixed,
        term_months: months_remaining,
        start_date,
        payment_amount: current_payment,
        extra_payment: Decimal::ZERO,
        linked_account: String::new(),
    };

    let new_loan = Loan {
        id: "new".into(),
        name: "Refinanced".into(),
        loan_type: LoanType::Personal,
        principal: remaining_balance,
        annual_rate: new_rate,
        rate_type: RateType::Fixed,
        term_months: new_term_months,
        start_date,
        payment_amount: new_payment,
        extra_payment: Decimal::ZERO,
        linked_account: String::new(),
    };

    let opts = ScheduleOptions::default();
    let current_schedule = generate_schedule(&current_loan, &opts)?;
    let new_schedule = generate_schedule(&new_loan, &opts)?;

    // Find break-even month by comparing cumulative costs
    let break_even_month = find_break_even(
        &current_schedule
            .entries
            .iter()
            .map(|e| e.payment)
            .collect::<Vec<_>>(),
        &new_schedule
            .entries
            .iter()
            .map(|e| e.payment)
            .collect::<Vec<_>>(),
        closing_costs,
    );

    let current_remaining_cost = current_schedule.total_paid;
    let new_total_cost = new_schedule.total_paid + closing_costs;
    let net_savings = round_currency(current_remaining_cost - new_total_cost);

    Ok(RefinanceComparison {
        new_payment,
        monthly_savings: round_currency(current_payment - new_payment),
        new_total_interest: new_schedule.total_interest,
        current_remaining_interest: current_schedule.total_interest,
        net_savings,
        closing_costs,
        break_even_month,
        new_payoff_date: new_schedule.payoff_date,
        new_term_months,
    })
}

/// Find the first month where cumulative cost of the new loan (including closing
/// costs) is less than cumulative cost of the current loan.
fn find_break_even(
    current_payments: &[Decimal],
    new_payments: &[Decimal],
    closing_costs: Decimal,
) -> Option<u32> {
    let mut cumulative_current = Decimal::ZERO;
    let mut cumulative_new = closing_costs;

    let max_months = current_payments.len().max(new_payments.len());

    for month in 0..max_months {
        cumulative_current += current_payments
            .get(month)
            .copied()
            .unwrap_or(Decimal::ZERO);
        cumulative_new += new_payments.get(month).copied().unwrap_or(Decimal::ZERO);

        if cumulative_new < cumulative_current {
            #[allow(clippy::cast_possible_truncation)]
            return Some((month + 1) as u32);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refinance_lower_rate_saves() {
        let result = compare_refinance(
            Decimal::new(180_000, 0), // remaining balance
            300,                      // 25 years remaining
            Decimal::new(65, 3),      // 6.5%
            Decimal::new(55, 3),      // 5.5%
            300,                      // same term
            Decimal::new(3000, 0),    // closing costs
            chrono::NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
        )
        .unwrap();

        assert!(result.monthly_savings > Decimal::ZERO);
        assert!(result.net_savings > Decimal::ZERO);
        assert!(result.break_even_month.is_some());
        assert!(result.new_total_interest < result.current_remaining_interest);
    }

    #[test]
    fn refinance_higher_rate_no_savings() {
        let result = compare_refinance(
            Decimal::new(180_000, 0),
            300,
            Decimal::new(55, 3), // 5.5% current
            Decimal::new(65, 3), // 6.5% new (higher!)
            300,
            Decimal::new(3000, 0),
            chrono::NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
        )
        .unwrap();

        assert!(result.monthly_savings < Decimal::ZERO);
        assert!(result.net_savings < Decimal::ZERO);
        // May or may not have a break-even depending on term changes
    }

    #[test]
    fn refinance_break_even_month() {
        let result = compare_refinance(
            Decimal::new(180_000, 0),
            300,
            Decimal::new(65, 3),
            Decimal::new(55, 3),
            300,
            Decimal::new(3000, 0),
            chrono::NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
        )
        .unwrap();

        let be = result.break_even_month.unwrap();
        // Break-even should be reasonable (within a few years)
        assert!(be > 0);
        assert!(be < 60); // should break even within 5 years
    }

    #[test]
    fn refinance_shorter_term() {
        let result = compare_refinance(
            Decimal::new(180_000, 0),
            300,                 // 25 years remaining
            Decimal::new(65, 3), // 6.5%
            Decimal::new(50, 3), // 5.0%
            180,                 // 15 year refi
            Decimal::new(3000, 0),
            chrono::NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
        )
        .unwrap();

        // Higher monthly payment but much less total interest
        assert!(result.monthly_savings < Decimal::ZERO); // payment goes up
        assert!(result.net_savings > Decimal::ZERO); // but total cost goes down
    }

    #[test]
    fn refinance_invalid_inputs() {
        let date = chrono::NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();
        assert!(
            compare_refinance(
                Decimal::ZERO,
                300,
                Decimal::new(65, 3),
                Decimal::new(55, 3),
                300,
                Decimal::new(3000, 0),
                date
            )
            .is_err()
        );
        assert!(
            compare_refinance(
                Decimal::new(180_000, 0),
                0,
                Decimal::new(65, 3),
                Decimal::new(55, 3),
                300,
                Decimal::new(3000, 0),
                date
            )
            .is_err()
        );
    }
}
