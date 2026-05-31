//! Amortization schedule generation for fixed and variable rate loans.

use chrono::Datelike;
use rust_decimal::Decimal;

use super::{
    AmortizationEntry, AmortizationSchedule, Loan, LoanError, ScheduleOptions, decimal_pow,
    round_currency,
};

/// Compute the monthly payment for a fixed-rate loan.
///
/// Formula: M = P × [r(1+r)^n] / [(1+r)^n − 1]
/// where P = principal, r = monthly rate, n = term in months.
///
/// Returns `Err` if inputs are invalid. Returns the exact principal/months
/// for zero-interest loans.
pub fn compute_monthly_payment(
    principal: Decimal,
    annual_rate: Decimal,
    term_months: u32,
) -> Result<Decimal, LoanError> {
    if term_months == 0 {
        return Err(LoanError::ZeroTerm);
    }
    if principal <= Decimal::ZERO {
        return Err(LoanError::NonPositivePrincipal);
    }
    if annual_rate < Decimal::ZERO {
        return Err(LoanError::NegativeRate);
    }

    if annual_rate.is_zero() {
        return Ok(round_currency(principal / Decimal::from(term_months)));
    }

    let r = annual_rate / Decimal::from(12);
    let one_plus_r_n = decimal_pow(Decimal::ONE + r, term_months);
    let payment = principal * (r * one_plus_r_n) / (one_plus_r_n - Decimal::ONE);

    Ok(round_currency(payment))
}

/// Generate a full amortization schedule.
///
/// Supports both fixed and variable rate loans. For variable rate loans,
/// pass rate adjustments via `options.rate_adjustments`.
pub fn generate_schedule(
    loan: &Loan,
    options: &ScheduleOptions,
) -> Result<AmortizationSchedule, LoanError> {
    validate_loan(loan)?;
    validate_options(options)?;

    let extra = options.extra_payment_override.unwrap_or(loan.extra_payment);

    let mut balance = loan.principal;
    let mut entries = Vec::new();
    let mut total_interest = Decimal::ZERO;
    let mut total_paid = Decimal::ZERO;
    let mut current_rate = loan.annual_rate;
    let mut current_payment = loan.payment_amount;
    let mut adj_iter = options.rate_adjustments.iter().peekable();

    // Cap at term_months * 2 to prevent runaway loops
    let max_months = loan.term_months * 2;

    for month in 1..=max_months {
        if balance <= Decimal::ZERO {
            break;
        }

        // Apply rate adjustment if due
        if let Some(adj) = adj_iter.peek()
            && adj.effective_month <= month
        {
            current_rate = adj.annual_rate;
            // Recalculate payment for remaining term at new rate
            let remaining_months = loan.term_months.saturating_sub(month - 1);
            if remaining_months > 0 {
                current_payment = compute_monthly_payment(balance, current_rate, remaining_months)?;
            }
            adj_iter.next();
        }

        let monthly_rate = current_rate / Decimal::from(12);
        let interest = round_currency(balance * monthly_rate);

        // Principal portion of regular payment
        let regular_principal = if current_payment > interest {
            current_payment - interest
        } else {
            Decimal::ZERO
        };

        // Extra payment, clamped to remaining balance after regular principal
        let remaining_after_regular = balance - regular_principal;
        let applied_extra = if remaining_after_regular > Decimal::ZERO {
            extra.min(remaining_after_regular)
        } else {
            Decimal::ZERO
        };

        // Total principal reduction
        let total_principal = regular_principal + applied_extra;

        // For final payment, clamp to remaining balance
        let (final_principal, final_interest, final_extra, final_payment) =
            if total_principal >= balance {
                let p = balance;
                let e = if p > regular_principal {
                    round_currency(p - regular_principal)
                } else {
                    Decimal::ZERO
                };
                let rp = round_currency(p - e);
                (rp, interest, e, interest + p)
            } else {
                (
                    round_currency(regular_principal),
                    interest,
                    round_currency(applied_extra),
                    round_currency(current_payment + applied_extra),
                )
            };

        balance = round_currency(balance - final_principal - final_extra);
        if balance < Decimal::ZERO {
            balance = Decimal::ZERO;
        }

        let date = advance_months(loan.start_date, month);

        total_interest += final_interest;
        total_paid += final_payment;

        entries.push(AmortizationEntry {
            month,
            date,
            payment: final_payment,
            principal: final_principal,
            interest: final_interest,
            extra_payment: final_extra,
            balance,
        });

        if balance.is_zero() {
            break;
        }
    }

    let payoff_date = entries.last().map_or(loan.start_date, |e| e.date);
    #[allow(clippy::cast_possible_truncation)]
    let months_to_payoff = entries.len() as u32;

    Ok(AmortizationSchedule {
        entries,
        total_interest: round_currency(total_interest),
        total_paid: round_currency(total_paid),
        payoff_date,
        months_to_payoff,
    })
}

fn validate_loan(loan: &Loan) -> Result<(), LoanError> {
    if loan.term_months == 0 {
        return Err(LoanError::ZeroTerm);
    }
    if loan.principal <= Decimal::ZERO {
        return Err(LoanError::NonPositivePrincipal);
    }
    if loan.annual_rate < Decimal::ZERO {
        return Err(LoanError::NegativeRate);
    }
    if loan.payment_amount <= Decimal::ZERO {
        return Err(LoanError::NonPositivePayment);
    }
    Ok(())
}

fn validate_options(options: &ScheduleOptions) -> Result<(), LoanError> {
    for window in options.rate_adjustments.windows(2) {
        if window[0].effective_month >= window[1].effective_month {
            return Err(LoanError::UnsortedAdjustments);
        }
    }
    Ok(())
}

fn advance_months(start: chrono::NaiveDate, months: u32) -> chrono::NaiveDate {
    let total_months = start.month0() + months;
    #[allow(clippy::cast_possible_wrap)]
    let year = start.year() + (total_months / 12) as i32;
    let month = (total_months % 12) + 1;
    // Clamp day to the last valid day of the target month
    let max_day = days_in_month(year, month);
    let day = start.day().min(max_day);
    chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap_or(start)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    chrono::NaiveDate::from_ymd_opt(
        if month == 12 { year + 1 } else { year },
        if month == 12 { 1 } else { month + 1 },
        1,
    )
    .map_or(30, |d| d.pred_opt().map_or(30, |d| d.day()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loans::{LoanType, RateAdjustment, RateType};

    fn mortgage_loan() -> Loan {
        let payment = compute_monthly_payment(
            Decimal::new(200_000, 0),
            Decimal::new(65, 3), // 6.5%
            360,
        )
        .unwrap();
        Loan {
            id: "m1".into(),
            name: "Home Mortgage".into(),
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

    fn auto_loan() -> Loan {
        let payment = compute_monthly_payment(
            Decimal::new(25_000, 0),
            Decimal::new(5, 2), // 5%
            60,
        )
        .unwrap();
        Loan {
            id: "a1".into(),
            name: "Car Loan".into(),
            loan_type: LoanType::Auto,
            principal: Decimal::new(25_000, 0),
            annual_rate: Decimal::new(5, 2),
            rate_type: RateType::Fixed,
            term_months: 60,
            start_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            payment_amount: payment,
            extra_payment: Decimal::ZERO,
            linked_account: "Liabilities:AutoLoan".into(),
        }
    }

    #[test]
    fn monthly_payment_30yr_mortgage() {
        let payment =
            compute_monthly_payment(Decimal::new(200_000, 0), Decimal::new(65, 3), 360).unwrap();
        // Expected: ~$1,264.14
        assert_eq!(payment, Decimal::new(126_414, 2));
    }

    #[test]
    fn monthly_payment_5yr_auto() {
        let payment =
            compute_monthly_payment(Decimal::new(25_000, 0), Decimal::new(5, 2), 60).unwrap();
        // Expected: ~$471.78
        assert_eq!(payment, Decimal::new(47178, 2));
    }

    #[test]
    fn zero_interest_payment() {
        let payment = compute_monthly_payment(Decimal::new(12_000, 0), Decimal::ZERO, 12).unwrap();
        assert_eq!(payment, Decimal::new(1000, 0));
    }

    #[test]
    fn invalid_inputs() {
        assert!(compute_monthly_payment(Decimal::new(1000, 0), Decimal::new(5, 2), 0).is_err());
        assert!(compute_monthly_payment(Decimal::ZERO, Decimal::new(5, 2), 12).is_err());
        assert!(compute_monthly_payment(Decimal::new(1000, 0), Decimal::new(-1, 1), 12).is_err());
    }

    #[test]
    fn schedule_fixed_rate_mortgage() {
        let loan = mortgage_loan();
        let schedule = generate_schedule(&loan, &ScheduleOptions::default()).unwrap();

        assert_eq!(schedule.months_to_payoff, 360);
        // Final balance should be zero
        assert_eq!(schedule.entries.last().unwrap().balance, Decimal::ZERO);
        // Total interest should be roughly $255K for a 200K mortgage at 6.5%
        assert!(schedule.total_interest > Decimal::new(250_000, 0));
        assert!(schedule.total_interest < Decimal::new(260_000, 0));
    }

    #[test]
    fn schedule_with_extra_payments() {
        let mut loan = mortgage_loan();
        loan.extra_payment = Decimal::new(200, 0);
        let schedule = generate_schedule(&loan, &ScheduleOptions::default()).unwrap();

        // Should pay off faster than 360 months
        assert!(schedule.months_to_payoff < 360);
        // Final balance should be zero
        assert_eq!(schedule.entries.last().unwrap().balance, Decimal::ZERO);
    }

    #[test]
    fn schedule_auto_loan() {
        let loan = auto_loan();
        let schedule = generate_schedule(&loan, &ScheduleOptions::default()).unwrap();

        // May be 60 or 61 months due to payment rounding (small final residual)
        assert!(schedule.months_to_payoff >= 60);
        assert!(schedule.months_to_payoff <= 61);
        assert_eq!(schedule.entries.last().unwrap().balance, Decimal::ZERO);
    }

    #[test]
    fn first_payment_breakdown() {
        let loan = mortgage_loan();
        let schedule = generate_schedule(&loan, &ScheduleOptions::default()).unwrap();
        let first = &schedule.entries[0];

        // First month interest on $200K at 6.5%: 200000 * 0.065/12 = $1,083.33
        assert_eq!(first.interest, Decimal::new(108_333, 2));
        assert_eq!(first.month, 1);
    }

    #[test]
    fn variable_rate_schedule() {
        let loan = mortgage_loan();
        let options = ScheduleOptions {
            rate_adjustments: vec![RateAdjustment {
                effective_month: 61,
                annual_rate: Decimal::new(75, 3), // 7.5% after 5 years
            }],
            extra_payment_override: None,
        };
        let schedule = generate_schedule(&loan, &options).unwrap();

        // Should still pay off (capped at term * 2)
        assert_eq!(schedule.entries.last().unwrap().balance, Decimal::ZERO);
        // Higher rate means more total interest
        let fixed_schedule = generate_schedule(&loan, &ScheduleOptions::default()).unwrap();
        assert!(schedule.total_interest > fixed_schedule.total_interest);
    }

    #[test]
    fn zero_interest_schedule() {
        let loan = Loan {
            id: "z1".into(),
            name: "Interest-free".into(),
            loan_type: LoanType::Personal,
            principal: Decimal::new(12_000, 0),
            annual_rate: Decimal::ZERO,
            rate_type: RateType::Fixed,
            term_months: 12,
            start_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            payment_amount: Decimal::new(1000, 0),
            extra_payment: Decimal::ZERO,
            linked_account: "Liabilities:Personal".into(),
        };
        let schedule = generate_schedule(&loan, &ScheduleOptions::default()).unwrap();

        assert_eq!(schedule.months_to_payoff, 12);
        assert_eq!(schedule.total_interest, Decimal::ZERO);
        assert_eq!(schedule.entries.last().unwrap().balance, Decimal::ZERO);
    }
}
