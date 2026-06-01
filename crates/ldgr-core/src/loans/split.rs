//! Payment auto-split: compute principal and interest portions of a loan payment.

use rust_decimal::Decimal;

use super::{LoanError, round_currency};

/// The result of splitting a loan payment into principal and interest.
#[derive(Debug, Clone)]
pub struct PaymentSplit {
    /// Interest portion of the payment.
    pub interest: Decimal,
    /// Principal portion of the payment.
    pub principal: Decimal,
    /// Extra principal applied beyond the regular payment.
    pub extra_principal: Decimal,
    /// Remaining loan balance after this payment.
    pub remaining_balance: Decimal,
    /// Whether this is the final payment (balance reaches zero).
    pub is_final_payment: bool,
}

/// Split a loan payment into principal and interest portions.
///
/// # Arguments
/// - `current_balance` — outstanding principal before this payment
/// - `annual_rate` — annual interest rate as decimal fraction (e.g. 0.065)
/// - `payment_amount` — total payment being made (principal + interest)
/// - `extra_payment` — additional principal payment beyond the regular amount
///
/// Handles edge cases:
/// - Final payment: clamps total reduction to remaining balance
/// - Underpayment: payment less than interest (warns via `principal` = 0 or negative)
/// - Overpayment: extra payment exceeds remaining balance after regular principal
pub fn split_payment(
    current_balance: Decimal,
    annual_rate: Decimal,
    payment_amount: Decimal,
    extra_payment: Decimal,
) -> Result<PaymentSplit, LoanError> {
    if current_balance <= Decimal::ZERO {
        return Ok(PaymentSplit {
            interest: Decimal::ZERO,
            principal: Decimal::ZERO,
            extra_principal: Decimal::ZERO,
            remaining_balance: Decimal::ZERO,
            is_final_payment: true,
        });
    }
    if annual_rate < Decimal::ZERO {
        return Err(LoanError::NegativeRate);
    }
    if payment_amount <= Decimal::ZERO {
        return Err(LoanError::NonPositivePayment);
    }

    let monthly_rate = annual_rate / Decimal::from(12);
    let interest = round_currency(current_balance * monthly_rate);

    // Regular principal portion
    let regular_principal = if payment_amount > interest {
        round_currency(payment_amount - interest)
    } else {
        // Payment doesn't cover interest — no principal reduction
        Decimal::ZERO
    };

    // Clamp regular principal to balance
    let regular_principal = regular_principal.min(current_balance);

    // Extra principal, clamped to remaining balance after regular principal
    let remaining_after_regular = current_balance - regular_principal;
    let applied_extra = round_currency(
        extra_payment
            .min(remaining_after_regular)
            .max(Decimal::ZERO),
    );

    let total_reduction = regular_principal + applied_extra;
    let remaining_balance = round_currency(current_balance - total_reduction);
    let is_final = remaining_balance <= Decimal::ZERO;

    Ok(PaymentSplit {
        interest,
        principal: regular_principal,
        extra_principal: applied_extra,
        remaining_balance: remaining_balance.max(Decimal::ZERO),
        is_final_payment: is_final,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_split() {
        let result = split_payment(
            Decimal::new(200_000, 0), // $200,000 balance
            Decimal::new(65, 3),      // 6.5%
            Decimal::new(126_414, 2), // $1,264.14 monthly payment
            Decimal::ZERO,
        )
        .unwrap();

        // Interest: 200000 * 0.065/12 = $1,083.33
        assert_eq!(result.interest, Decimal::new(108_333, 2));
        // Principal: 1264.14 - 1083.33 = $180.81
        assert_eq!(result.principal, Decimal::new(18081, 2));
        assert_eq!(result.extra_principal, Decimal::ZERO);
        assert!(!result.is_final_payment);
        // Verify: principal + interest = payment
        assert_eq!(result.principal + result.interest, Decimal::new(126_414, 2));
    }

    #[test]
    fn split_with_extra_payment() {
        let result = split_payment(
            Decimal::new(200_000, 0),
            Decimal::new(65, 3),
            Decimal::new(126_414, 2),
            Decimal::new(200, 0), // $200 extra
        )
        .unwrap();

        assert_eq!(result.interest, Decimal::new(108_333, 2));
        assert_eq!(result.principal, Decimal::new(18081, 2));
        assert_eq!(result.extra_principal, Decimal::new(200, 0));
        assert!(!result.is_final_payment);
    }

    #[test]
    fn final_payment() {
        let result = split_payment(
            Decimal::new(500, 0),     // only $500 left
            Decimal::new(65, 3),      // 6.5%
            Decimal::new(126_414, 2), // regular payment overshoots
            Decimal::ZERO,
        )
        .unwrap();

        // Interest: 500 * 0.065/12 = $2.71
        assert_eq!(result.interest, Decimal::new(271, 2));
        // Principal should be clamped to balance
        assert_eq!(result.remaining_balance, Decimal::ZERO);
        assert!(result.is_final_payment);
    }

    #[test]
    fn extra_exceeds_balance() {
        let result = split_payment(
            Decimal::new(300, 0), // $300 balance
            Decimal::new(65, 3),
            Decimal::new(126_414, 2),
            Decimal::new(10_000, 0), // huge extra payment
        )
        .unwrap();

        assert_eq!(result.remaining_balance, Decimal::ZERO);
        assert!(result.is_final_payment);
    }

    #[test]
    fn zero_balance() {
        let result = split_payment(
            Decimal::ZERO,
            Decimal::new(65, 3),
            Decimal::new(126_414, 2),
            Decimal::ZERO,
        )
        .unwrap();

        assert_eq!(result.interest, Decimal::ZERO);
        assert_eq!(result.principal, Decimal::ZERO);
        assert!(result.is_final_payment);
    }

    #[test]
    fn zero_interest_rate() {
        let result = split_payment(
            Decimal::new(12_000, 0),
            Decimal::ZERO,
            Decimal::new(1000, 0),
            Decimal::ZERO,
        )
        .unwrap();

        assert_eq!(result.interest, Decimal::ZERO);
        assert_eq!(result.principal, Decimal::new(1000, 0));
        assert_eq!(result.remaining_balance, Decimal::new(11_000, 0));
    }

    #[test]
    fn payment_less_than_interest() {
        // Payment of $500 but interest is $1,083.33
        let result = split_payment(
            Decimal::new(200_000, 0),
            Decimal::new(65, 3),
            Decimal::new(500, 0),
            Decimal::ZERO,
        )
        .unwrap();

        assert_eq!(result.interest, Decimal::new(108_333, 2));
        // No principal reduction
        assert_eq!(result.principal, Decimal::ZERO);
        // Balance unchanged
        assert_eq!(result.remaining_balance, Decimal::new(200_000, 0));
    }

    #[test]
    fn invalid_inputs() {
        assert!(
            split_payment(
                Decimal::new(1000, 0),
                Decimal::new(-1, 1),
                Decimal::new(100, 0),
                Decimal::ZERO,
            )
            .is_err()
        );
        assert!(
            split_payment(
                Decimal::new(1000, 0),
                Decimal::new(5, 2),
                Decimal::ZERO,
                Decimal::ZERO,
            )
            .is_err()
        );
    }
}
