//! Budget engine: envelope and zero-based budget computation.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Budget method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetMethod {
    /// Fixed amounts per category, unspent rolls over.
    Envelope,
    /// Every dollar assigned, allocations must equal income.
    ZeroBased,
}

/// A budget definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub id: String,
    pub name: String,
    pub method: BudgetMethod,
    pub period: BudgetPeriod,
    pub allocations: Vec<BudgetAllocation>,
}

/// Budget period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetPeriod {
    Monthly,
    Weekly,
    Quarterly,
    Annual,
}

/// A single budget category allocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAllocation {
    pub account: String,
    pub amount: Decimal,
    pub rollover: bool,
}

/// Budget vs actual result for a single category.
#[derive(Debug, Clone)]
pub struct BudgetVsActual {
    pub account: String,
    pub budgeted: Decimal,
    pub actual: Decimal,
    pub remaining: Decimal,
    pub percent_used: Decimal,
    pub over_budget: bool,
}

/// Full budget report.
#[derive(Debug, Clone)]
pub struct BudgetReport {
    pub name: String,
    pub method: BudgetMethod,
    pub categories: Vec<BudgetVsActual>,
    pub total_budgeted: Decimal,
    pub total_actual: Decimal,
    pub total_remaining: Decimal,
}

/// Compute budget vs actual from a budget definition and actual spending.
///
/// `actuals` maps account name → total spent (as positive Decimal).
/// `carryover` maps account name → unspent from previous period (envelope only).
pub fn compute_budget_vs_actual(
    budget: &Budget,
    actuals: &BTreeMap<String, Decimal>,
    carryover: &BTreeMap<String, Decimal>,
) -> BudgetReport {
    let mut categories = Vec::new();
    let mut total_budgeted = Decimal::ZERO;
    let mut total_actual = Decimal::ZERO;

    for alloc in &budget.allocations {
        let carry = if alloc.rollover && budget.method == BudgetMethod::Envelope {
            carryover
                .get(&alloc.account)
                .copied()
                .unwrap_or(Decimal::ZERO)
        } else {
            Decimal::ZERO
        };

        let budgeted = alloc.amount + carry;
        let actual = actuals
            .get(&alloc.account)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let remaining = budgeted - actual;
        let percent_used = if budgeted.is_zero() {
            Decimal::ZERO
        } else {
            (actual / budgeted) * Decimal::new(100, 0)
        };

        categories.push(BudgetVsActual {
            account: alloc.account.clone(),
            budgeted,
            actual,
            remaining,
            percent_used,
            over_budget: actual > budgeted,
        });

        total_budgeted += budgeted;
        total_actual += actual;
    }

    BudgetReport {
        name: budget.name.clone(),
        method: budget.method,
        categories,
        total_budgeted,
        total_actual,
        total_remaining: total_budgeted - total_actual,
    }
}

/// Validate a zero-based budget: allocations must equal the given income.
pub fn validate_zero_based(budget: &Budget, income: Decimal) -> Result<(), String> {
    if budget.method != BudgetMethod::ZeroBased {
        return Ok(());
    }
    let total: Decimal = budget.allocations.iter().map(|a| a.amount).sum();
    if total != income {
        return Err(format!(
            "Zero-based budget allocations ({total}) do not equal income ({income})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_budget() -> Budget {
        Budget {
            id: "b1".into(),
            name: "Monthly".into(),
            method: BudgetMethod::Envelope,
            period: BudgetPeriod::Monthly,
            allocations: vec![
                BudgetAllocation {
                    account: "Expenses:Food".into(),
                    amount: Decimal::new(500, 0),
                    rollover: true,
                },
                BudgetAllocation {
                    account: "Expenses:Transport".into(),
                    amount: Decimal::new(200, 0),
                    rollover: false,
                },
                BudgetAllocation {
                    account: "Expenses:Entertainment".into(),
                    amount: Decimal::new(100, 0),
                    rollover: true,
                },
            ],
        }
    }

    #[test]
    fn budget_vs_actual_basic() {
        let budget = sample_budget();
        let mut actuals = BTreeMap::new();
        actuals.insert("Expenses:Food".into(), Decimal::new(420, 0));
        actuals.insert("Expenses:Transport".into(), Decimal::new(180, 0));

        let report = compute_budget_vs_actual(&budget, &actuals, &BTreeMap::new());
        assert_eq!(report.categories[0].remaining, Decimal::new(80, 0));
        assert!(!report.categories[0].over_budget);
        assert_eq!(report.categories[2].actual, Decimal::ZERO);
    }

    #[test]
    fn over_budget_detection() {
        let budget = sample_budget();
        let mut actuals = BTreeMap::new();
        actuals.insert("Expenses:Food".into(), Decimal::new(600, 0));

        let report = compute_budget_vs_actual(&budget, &actuals, &BTreeMap::new());
        assert!(report.categories[0].over_budget);
        assert_eq!(report.categories[0].remaining, Decimal::new(-100, 0));
    }

    #[test]
    fn envelope_rollover() {
        let budget = sample_budget();
        let mut carryover = BTreeMap::new();
        carryover.insert("Expenses:Food".into(), Decimal::new(50, 0));

        let report = compute_budget_vs_actual(&budget, &BTreeMap::new(), &carryover);
        // Food: 500 + 50 carryover = 550 budgeted
        assert_eq!(report.categories[0].budgeted, Decimal::new(550, 0));
        // Transport: no rollover, stays at 200
        assert_eq!(report.categories[1].budgeted, Decimal::new(200, 0));
    }

    #[test]
    fn zero_based_validation() {
        let budget = Budget {
            id: "zb".into(),
            name: "Zero".into(),
            method: BudgetMethod::ZeroBased,
            period: BudgetPeriod::Monthly,
            allocations: vec![
                BudgetAllocation {
                    account: "A".into(),
                    amount: Decimal::new(3000, 0),
                    rollover: false,
                },
                BudgetAllocation {
                    account: "B".into(),
                    amount: Decimal::new(2000, 0),
                    rollover: false,
                },
            ],
        };
        assert!(validate_zero_based(&budget, Decimal::new(5000, 0)).is_ok());
        assert!(validate_zero_based(&budget, Decimal::new(4000, 0)).is_err());
    }
}
