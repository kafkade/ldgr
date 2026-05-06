//! Goal tracking engine with linear and compound projections.

use chrono::Datelike;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Goal type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalType {
    Savings,
    DebtPayoff,
    Investment,
    EmergencyFund,
    Retirement,
    Custom,
}

/// A financial goal definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub name: String,
    pub goal_type: GoalType,
    pub target_amount: Decimal,
    pub target_date: Option<String>,
    pub linked_account: Option<String>,
}

/// Goal progress snapshot.
#[derive(Debug, Clone)]
pub struct GoalProgress {
    pub goal: Goal,
    pub current_amount: Decimal,
    pub remaining: Decimal,
    pub percent_complete: Decimal,
    pub on_track: bool,
    pub projected_date: Option<String>,
}

/// What-if scenario result.
#[derive(Debug, Clone)]
pub struct WhatIfResult {
    pub monthly_contribution: Decimal,
    pub months_to_goal: u32,
    pub projected_date: String,
}

/// Compute goal progress given the current balance.
pub fn compute_progress(
    goal: &Goal,
    current_amount: Decimal,
    monthly_contribution: Decimal,
    current_date: &str,
) -> GoalProgress {
    let remaining = goal.target_amount - current_amount;
    let percent = if goal.target_amount.is_zero() {
        Decimal::new(100, 0)
    } else {
        (current_amount / goal.target_amount) * Decimal::new(100, 0)
    };

    let projected_date = if monthly_contribution > Decimal::ZERO && remaining > Decimal::ZERO {
        let months = (remaining / monthly_contribution).ceil();
        project_date(current_date, months)
    } else if remaining <= Decimal::ZERO {
        Some(current_date.to_string())
    } else {
        None
    };

    let on_track = match (&goal.target_date, &projected_date) {
        (Some(target), Some(projected)) => projected.as_str() <= target.as_str(),
        (None, _) => true,
        (Some(_), None) => false,
    };

    GoalProgress {
        goal: goal.clone(),
        current_amount,
        remaining: remaining.max(Decimal::ZERO),
        percent_complete: percent.min(Decimal::new(100, 0)),
        on_track,
        projected_date,
    }
}

/// Run a what-if scenario: how long to reach the goal at a given monthly rate?
pub fn what_if(
    goal: &Goal,
    current_amount: Decimal,
    monthly_contribution: Decimal,
    current_date: &str,
) -> WhatIfResult {
    let remaining = goal.target_amount - current_amount;

    let months = if monthly_contribution > Decimal::ZERO && remaining > Decimal::ZERO {
        (remaining / monthly_contribution).ceil()
    } else if remaining <= Decimal::ZERO {
        Decimal::ZERO
    } else {
        Decimal::new(9999, 0)
    };

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let months_u32 = months.to_string().parse::<u32>().unwrap_or(9999);

    let projected_date = project_date(current_date, months).unwrap_or_else(|| "unknown".into());

    WhatIfResult {
        monthly_contribution,
        months_to_goal: months_u32,
        projected_date,
    }
}

/// Required monthly contribution to reach a goal by a target date.
pub fn required_contribution(
    goal: &Goal,
    current_amount: Decimal,
    current_date: &str,
) -> Option<Decimal> {
    let target_date = goal.target_date.as_deref()?;
    let months = months_between(current_date, target_date)?;
    if months <= 0 {
        return None;
    }
    let remaining = goal.target_amount - current_amount;
    if remaining <= Decimal::ZERO {
        return Some(Decimal::ZERO);
    }
    Some(remaining / Decimal::from(months))
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn project_date(from: &str, months: Decimal) -> Option<String> {
    let date = chrono::NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let m = months.to_string().parse::<i32>().unwrap_or(0);
    let result = date.checked_add_months(chrono::Months::new(m.unsigned_abs()))?;
    Some(result.format("%Y-%m-%d").to_string())
}

fn months_between(from: &str, to: &str) -> Option<i32> {
    let d1 = chrono::NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let d2 = chrono::NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    #[allow(clippy::cast_possible_wrap)]
    let months = (d2.year() - d1.year()) * 12 + (d2.month() as i32 - d1.month() as i32);
    Some(months)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_goal() -> Goal {
        Goal {
            id: "g1".into(),
            name: "Emergency Fund".into(),
            goal_type: GoalType::EmergencyFund,
            target_amount: Decimal::new(10000, 0),
            target_date: Some("2025-12-31".into()),
            linked_account: Some("Assets:Savings".into()),
        }
    }

    #[test]
    fn progress_calculation() {
        let goal = sample_goal();
        let progress = compute_progress(
            &goal,
            Decimal::new(4000, 0),
            Decimal::new(500, 0),
            "2024-06-01",
        );
        assert_eq!(progress.percent_complete, Decimal::new(40, 0));
        assert_eq!(progress.remaining, Decimal::new(6000, 0));
        assert!(progress.projected_date.is_some());
    }

    #[test]
    fn goal_already_met() {
        let goal = sample_goal();
        let progress = compute_progress(&goal, Decimal::new(15000, 0), Decimal::ZERO, "2024-06-01");
        assert_eq!(progress.percent_complete, Decimal::new(100, 0));
        assert_eq!(progress.remaining, Decimal::ZERO);
    }

    #[test]
    fn what_if_scenario() {
        let goal = sample_goal();
        let result = what_if(
            &goal,
            Decimal::new(4000, 0),
            Decimal::new(1000, 0),
            "2024-06-01",
        );
        // 6000 remaining / 1000/month = 6 months
        assert_eq!(result.months_to_goal, 6);
    }

    #[test]
    fn required_contribution_calculation() {
        let goal = sample_goal();
        let required = required_contribution(&goal, Decimal::new(4000, 0), "2024-06-01");
        // 6000 remaining, ~18 months to Dec 2025 → ~333/month
        assert!(required.is_some());
        let r = required.unwrap();
        assert!(r > Decimal::new(300, 0));
        assert!(r < Decimal::new(400, 0));
    }

    #[test]
    fn no_target_date_always_on_track() {
        let mut goal = sample_goal();
        goal.target_date = None;
        let progress = compute_progress(
            &goal,
            Decimal::new(1000, 0),
            Decimal::new(100, 0),
            "2024-06-01",
        );
        assert!(progress.on_track);
    }
}
