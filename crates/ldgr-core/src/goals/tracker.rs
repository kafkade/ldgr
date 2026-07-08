//! Goal tracking engine with linear and compound projections.
//!
//! # Advanced projections
//!
//! On top of the basic linear tracking ([`compute_progress`], [`what_if`],
//! [`required_contribution`]) this module offers:
//!
//! - **Multi-scenario bands** ([`project_band`]) — model a return-rate range
//!   (pessimistic / expected / optimistic) instead of a single rate.
//! - **Inflation-adjusted targets** ([`inflation_adjusted_target`],
//!   [`project_with_inflation`]) — chase a *nominal* future target so a goal's
//!   purchasing power is preserved.
//! - **Variable contribution schedules** ([`ContributionSchedule`],
//!   [`project_schedule`]) — step-ups and one-off lump sums over time.
//! - **Shared-budget allocation** ([`allocate_budget`]) — divide a single
//!   monthly contribution budget across competing goals.
//! - **Linked-account actuals** ([`estimate_monthly_rate_from_history`]) —
//!   derive a contribution estimate from real balance history.
//!
//! ## Compounding convention
//!
//! To stay deterministic and float-free, growth and inflation use a **nominal
//! monthly rate = annual rate / 12** (APR compounded monthly), matching the
//! `loans` module. Projections are month-by-month simulations bounded by
//! [`MAX_PROJECTION_MONTHS`]; an unreachable goal reports `reached = false`
//! rather than looping forever.

use chrono::Datelike;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Hard cap on projection horizon (100 years) so simulations always terminate.
pub const MAX_PROJECTION_MONTHS: u32 = 1200;

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

// ── Advanced projections ────────────────────────────────────────────────────────

/// Convert an annual rate to the nominal monthly rate (annual / 12).
fn monthly_rate(annual_rate: Decimal) -> Decimal {
    annual_rate / Decimal::from(12)
}

/// Compute `base^exp` via square-and-multiply (integer exponent, float-free).
fn decimal_pow(base: Decimal, exp: u32) -> Decimal {
    let mut result = Decimal::ONE;
    let mut b = base;
    let mut e = exp;
    while e > 0 {
        if e % 2 == 1 {
            result *= b;
        }
        b *= b;
        e /= 2;
    }
    result
}

// ── Item 1: multi-scenario projection bands ─────────────────────────────────────

/// A range of annual return rates for optimistic / expected / pessimistic paths.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ScenarioRates {
    /// Conservative annual return (e.g. `0.02` for 2%).
    pub pessimistic: Decimal,
    /// Central/expected annual return.
    pub expected: Decimal,
    /// Bullish annual return.
    pub optimistic: Decimal,
}

impl Default for ScenarioRates {
    /// Sensible defaults: 2% / 5% / 8% annual.
    fn default() -> Self {
        Self {
            pessimistic: Decimal::new(2, 2),
            expected: Decimal::new(5, 2),
            optimistic: Decimal::new(8, 2),
        }
    }
}

/// Projection outcome for a single return-rate scenario.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioProjection {
    /// The annual return rate this projection assumed.
    pub annual_rate: Decimal,
    /// Whether the goal is reached within [`MAX_PROJECTION_MONTHS`].
    pub reached: bool,
    /// Months until the goal is met, if reachable.
    pub months_to_goal: Option<u32>,
    /// Calendar date the goal is projected to be met, if reachable.
    pub projected_date: Option<String>,
    /// Balance at completion (or at the horizon if unreachable).
    pub final_balance: Decimal,
}

/// Optimistic / expected / pessimistic projections for one goal.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectionBand {
    pub pessimistic: ScenarioProjection,
    pub expected: ScenarioProjection,
    pub optimistic: ScenarioProjection,
}

/// Project a goal under a single annual return rate with a flat contribution.
pub fn project_with_return(
    goal: &Goal,
    current_amount: Decimal,
    monthly_contribution: Decimal,
    annual_rate: Decimal,
    current_date: &str,
    max_months: u32,
) -> ScenarioProjection {
    let schedule = ContributionSchedule::flat(monthly_contribution);
    let outcome = simulate_schedule(
        goal.target_amount,
        current_amount,
        &schedule,
        annual_rate,
        Decimal::ZERO,
        max_months,
    );
    scenario_from_outcome(annual_rate, current_date, &outcome)
}

/// Project a goal across a full [`ScenarioRates`] band.
pub fn project_band(
    goal: &Goal,
    current_amount: Decimal,
    monthly_contribution: Decimal,
    rates: ScenarioRates,
    current_date: &str,
) -> ProjectionBand {
    ProjectionBand {
        pessimistic: project_with_return(
            goal,
            current_amount,
            monthly_contribution,
            rates.pessimistic,
            current_date,
            MAX_PROJECTION_MONTHS,
        ),
        expected: project_with_return(
            goal,
            current_amount,
            monthly_contribution,
            rates.expected,
            current_date,
            MAX_PROJECTION_MONTHS,
        ),
        optimistic: project_with_return(
            goal,
            current_amount,
            monthly_contribution,
            rates.optimistic,
            current_date,
            MAX_PROJECTION_MONTHS,
        ),
    }
}

// ── Item 2: inflation-adjusted targets ──────────────────────────────────────────

/// A goal target expressed in both today's (real) and future (nominal) terms.
#[derive(Debug, Clone, Serialize)]
pub struct InflationAdjustedTarget {
    /// The target in today's purchasing power (unchanged from the goal).
    pub real_target: Decimal,
    /// The target grown by inflation over `months`.
    pub nominal_target: Decimal,
    /// Horizon in months over which inflation was applied.
    pub months: u32,
    /// Annual inflation rate used.
    pub annual_inflation: Decimal,
}

/// Grow an amount by inflation over a number of months (nominal future value).
#[must_use]
pub fn inflate_target(target: Decimal, annual_inflation: Decimal, months: u32) -> Decimal {
    let mi = monthly_rate(annual_inflation);
    target * decimal_pow(Decimal::ONE + mi, months)
}

/// Compute the inflation-adjusted (nominal) target for a goal over `months`.
#[must_use]
pub fn inflation_adjusted_target(
    goal: &Goal,
    annual_inflation: Decimal,
    months: u32,
) -> InflationAdjustedTarget {
    InflationAdjustedTarget {
        real_target: goal.target_amount,
        nominal_target: inflate_target(goal.target_amount, annual_inflation, months),
        months,
        annual_inflation,
    }
}

/// Project a goal whose target grows with inflation while savings compound.
///
/// The target rises each month at `annual_inflation / 12`, so the projection
/// reflects the effort needed to preserve real purchasing power.
pub fn project_with_inflation(
    goal: &Goal,
    current_amount: Decimal,
    monthly_contribution: Decimal,
    annual_rate: Decimal,
    annual_inflation: Decimal,
    current_date: &str,
    max_months: u32,
) -> ScenarioProjection {
    let schedule = ContributionSchedule::flat(monthly_contribution);
    let outcome = simulate_schedule(
        goal.target_amount,
        current_amount,
        &schedule,
        annual_rate,
        annual_inflation,
        max_months,
    );
    scenario_from_outcome(annual_rate, current_date, &outcome)
}

// ── Item 3: variable contribution schedules ─────────────────────────────────────

/// A step change in the recurring monthly contribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributionStep {
    /// 1-based month from which `monthly_amount` applies.
    pub effective_month: u32,
    /// New recurring monthly contribution from `effective_month` onward.
    pub monthly_amount: Decimal,
}

/// A one-off extra deposit at a specific month.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LumpSum {
    /// 1-based month the lump sum lands.
    pub month: u32,
    /// Extra amount deposited that month.
    pub amount: Decimal,
}

/// A variable contribution plan: a base monthly amount plus step-ups and lump sums.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContributionSchedule {
    /// Baseline recurring monthly contribution before any steps.
    pub base_monthly: Decimal,
    /// Step changes to the recurring contribution over time.
    #[serde(default)]
    pub steps: Vec<ContributionStep>,
    /// One-off lump-sum deposits.
    #[serde(default)]
    pub lump_sums: Vec<LumpSum>,
}

impl ContributionSchedule {
    /// A flat schedule with a constant monthly contribution.
    #[must_use]
    pub fn flat(monthly: Decimal) -> Self {
        Self {
            base_monthly: monthly,
            steps: Vec::new(),
            lump_sums: Vec::new(),
        }
    }

    /// Recurring contribution active during `month` (1-based).
    ///
    /// The latest step whose `effective_month <= month` wins; otherwise the base.
    #[must_use]
    pub fn contribution_at_month(&self, month: u32) -> Decimal {
        self.steps
            .iter()
            .filter(|s| s.effective_month <= month)
            .max_by_key(|s| s.effective_month)
            .map_or(self.base_monthly, |s| s.monthly_amount)
    }

    /// Total lump-sum deposits landing in `month` (1-based).
    #[must_use]
    pub fn lump_sum_at_month(&self, month: u32) -> Decimal {
        self.lump_sums
            .iter()
            .filter(|l| l.month == month)
            .map(|l| l.amount)
            .sum()
    }
}

/// Projection outcome for a variable contribution schedule.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleProjection {
    /// Whether the goal is reached within the horizon.
    pub reached: bool,
    /// Months until the goal is met, if reachable.
    pub months_to_goal: Option<u32>,
    /// Calendar date the goal is projected to be met, if reachable.
    pub projected_date: Option<String>,
    /// Balance at completion (or at the horizon if unreachable).
    pub final_balance: Decimal,
    /// Total of all contributions and lump sums deposited over the simulation.
    pub total_contributed: Decimal,
}

/// Project a goal under a variable contribution schedule and return rate.
pub fn project_schedule(
    goal: &Goal,
    current_amount: Decimal,
    schedule: &ContributionSchedule,
    annual_rate: Decimal,
    current_date: &str,
    max_months: u32,
) -> ScheduleProjection {
    let outcome = simulate_schedule(
        goal.target_amount,
        current_amount,
        schedule,
        annual_rate,
        Decimal::ZERO,
        max_months,
    );
    let projected_date = outcome
        .months
        .and_then(|m| project_date(current_date, Decimal::from(m)));
    ScheduleProjection {
        reached: outcome.reached,
        months_to_goal: outcome.months,
        projected_date,
        final_balance: outcome.final_balance,
        total_contributed: outcome.total_contributed,
    }
}

// ── Shared month-by-month simulator ─────────────────────────────────────────────

/// Result of a schedule simulation.
struct SimOutcome {
    reached: bool,
    months: Option<u32>,
    final_balance: Decimal,
    total_contributed: Decimal,
}

/// Simulate savings growth toward a (possibly inflating) target.
///
/// Each month: the target grows by inflation, the balance grows by the return
/// rate, then the recurring contribution and any lump sum for that month are
/// added. Terminates when `balance >= target` or `max_months` is hit.
fn simulate_schedule(
    base_target: Decimal,
    start_balance: Decimal,
    schedule: &ContributionSchedule,
    annual_rate: Decimal,
    annual_inflation: Decimal,
    max_months: u32,
) -> SimOutcome {
    let mr = monthly_rate(annual_rate);
    let mi = monthly_rate(annual_inflation);
    let mut balance = start_balance;
    let mut target = base_target;
    let mut total_contributed = Decimal::ZERO;

    if balance >= target {
        return SimOutcome {
            reached: true,
            months: Some(0),
            final_balance: balance,
            total_contributed,
        };
    }

    let mut month = 0u32;
    while month < max_months {
        month += 1;
        if !mi.is_zero() {
            target += target * mi;
        }
        balance += balance * mr;
        let deposit = schedule.contribution_at_month(month) + schedule.lump_sum_at_month(month);
        balance += deposit;
        total_contributed += deposit;
        if balance >= target {
            return SimOutcome {
                reached: true,
                months: Some(month),
                final_balance: balance,
                total_contributed,
            };
        }
    }

    SimOutcome {
        reached: false,
        months: None,
        final_balance: balance,
        total_contributed,
    }
}

fn scenario_from_outcome(
    annual_rate: Decimal,
    current_date: &str,
    outcome: &SimOutcome,
) -> ScenarioProjection {
    let projected_date = outcome
        .months
        .and_then(|m| project_date(current_date, Decimal::from(m)));
    ScenarioProjection {
        annual_rate,
        reached: outcome.reached,
        months_to_goal: outcome.months,
        projected_date,
        final_balance: outcome.final_balance,
    }
}

// ── Item 4: shared-budget allocation across competing goals ──────────────────────

/// Strategy for dividing a shared monthly budget across goals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AllocationStrategy {
    /// Fund higher-`priority` goals first (ties broken by earliest target date).
    PriorityOrder,
    /// Fund goals with the earliest target date first.
    DeadlineFirst,
    /// Split proportionally to each goal's funding need.
    Proportional,
    /// Split the budget equally across all goals.
    EqualSplit,
}

/// Per-goal inputs for allocation.
#[derive(Debug, Clone)]
pub struct AllocationInput {
    pub goal_id: String,
    /// Amount still needed to reach the goal.
    pub remaining: Decimal,
    /// Monthly contribution required to hit the target date, if known.
    pub required_contribution: Option<Decimal>,
    /// Target date (`YYYY-MM-DD`), used for deadline ordering.
    pub target_date: Option<String>,
    /// Higher values are funded first under [`AllocationStrategy::PriorityOrder`].
    pub priority: i32,
}

impl AllocationInput {
    /// The monthly funding "need": the required contribution, else the remaining.
    fn need(&self) -> Decimal {
        self.required_contribution
            .unwrap_or(self.remaining)
            .max(Decimal::ZERO)
    }
}

/// How much of the shared budget a goal received.
#[derive(Debug, Clone, Serialize)]
pub struct GoalAllocation {
    pub goal_id: String,
    /// Amount of the monthly budget assigned to this goal.
    pub allocated: Decimal,
    /// The goal's required monthly contribution, if known.
    pub required: Option<Decimal>,
    /// Whether `allocated` covers `required`.
    pub meets_required: bool,
    /// Unmet required amount (`0` when met or unknown).
    pub shortfall: Decimal,
}

/// Allocate a shared monthly budget across competing goals.
///
/// The returned order matches the input order. Sequential strategies
/// (`PriorityOrder`, `DeadlineFirst`) fund each goal's need in turn until the
/// budget is exhausted; proportional and equal strategies divide across all.
#[must_use]
pub fn allocate_budget(
    goals: &[AllocationInput],
    total_monthly: Decimal,
    strategy: AllocationStrategy,
) -> Vec<GoalAllocation> {
    let budget = total_monthly.max(Decimal::ZERO);
    let allocations = match strategy {
        AllocationStrategy::PriorityOrder => allocate_sequential(goals, budget, |a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| cmp_target_date(a.target_date.as_deref(), b.target_date.as_deref()))
        }),
        AllocationStrategy::DeadlineFirst => allocate_sequential(goals, budget, |a, b| {
            cmp_target_date(a.target_date.as_deref(), b.target_date.as_deref())
                .then_with(|| b.priority.cmp(&a.priority))
        }),
        AllocationStrategy::Proportional => allocate_proportional(goals, budget),
        AllocationStrategy::EqualSplit => allocate_equal(goals, budget),
    };
    finalize_allocations(goals, &allocations)
}

fn cmp_target_date(a: Option<&str>, b: Option<&str>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// Sequential funding: sort by `order`, give each goal its need until dry.
fn allocate_sequential(
    goals: &[AllocationInput],
    budget: Decimal,
    order: impl Fn(&AllocationInput, &AllocationInput) -> std::cmp::Ordering,
) -> std::collections::HashMap<String, Decimal> {
    let mut indices: Vec<usize> = (0..goals.len()).collect();
    indices.sort_by(|&i, &j| order(&goals[i], &goals[j]));

    let mut remaining_budget = budget;
    let mut out = std::collections::HashMap::new();
    for i in indices {
        let need = goals[i].need();
        let give = need.min(remaining_budget).max(Decimal::ZERO);
        remaining_budget -= give;
        out.insert(goals[i].goal_id.clone(), give);
    }
    out
}

/// Proportional funding: split budget in proportion to each goal's need.
fn allocate_proportional(
    goals: &[AllocationInput],
    budget: Decimal,
) -> std::collections::HashMap<String, Decimal> {
    let total_need: Decimal = goals.iter().map(AllocationInput::need).sum();
    let mut out = std::collections::HashMap::new();
    if total_need.is_zero() {
        return allocate_equal(goals, budget);
    }
    for g in goals {
        let share = (budget * g.need() / total_need).round_dp(2);
        out.insert(g.goal_id.clone(), share);
    }
    out
}

/// Equal funding: divide the budget evenly across all goals.
fn allocate_equal(
    goals: &[AllocationInput],
    budget: Decimal,
) -> std::collections::HashMap<String, Decimal> {
    let mut out = std::collections::HashMap::new();
    if goals.is_empty() {
        return out;
    }
    let share = (budget / Decimal::from(goals.len() as u64)).round_dp(2);
    for g in goals {
        out.insert(g.goal_id.clone(), share);
    }
    out
}

fn finalize_allocations(
    goals: &[AllocationInput],
    allocations: &std::collections::HashMap<String, Decimal>,
) -> Vec<GoalAllocation> {
    goals
        .iter()
        .map(|g| {
            let allocated = allocations
                .get(&g.goal_id)
                .copied()
                .unwrap_or(Decimal::ZERO);
            let (meets_required, shortfall) = match g.required_contribution {
                Some(req) => (allocated >= req, (req - allocated).max(Decimal::ZERO)),
                None => (true, Decimal::ZERO),
            };
            GoalAllocation {
                goal_id: g.goal_id.clone(),
                allocated,
                required: g.required_contribution,
                meets_required,
                shortfall,
            }
        })
        .collect()
}

// ── Item 5: linked-account-driven actuals ───────────────────────────────────────

/// Estimate the average monthly balance change from real account history.
///
/// `points` must be chronological `(YYYY-MM-DD, balance)` samples. Returns the
/// average monthly delta between the first and last points, usable as an
/// empirical contribution estimate. Returns `None` when there are fewer than
/// two points or a non-positive span.
#[must_use]
pub fn estimate_monthly_rate_from_history(points: &[(String, Decimal)]) -> Option<Decimal> {
    if points.len() < 2 {
        return None;
    }
    let first = &points[0];
    let last = &points[points.len() - 1];
    let months = months_between(&first.0, &last.0)?;
    if months <= 0 {
        return None;
    }
    Some((last.1 - first.1) / Decimal::from(months))
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

    // ── Item 1: scenario bands ──────────────────────────────────────────────

    #[test]
    fn scenario_zero_rate_matches_linear() {
        let goal = sample_goal(); // target 10_000
        let p = project_with_return(
            &goal,
            Decimal::new(4000, 0),
            Decimal::new(1000, 0),
            Decimal::ZERO,
            "2024-06-01",
            MAX_PROJECTION_MONTHS,
        );
        // 6000 remaining / 1000 per month = 6 months, no growth.
        assert!(p.reached);
        assert_eq!(p.months_to_goal, Some(6));
        assert_eq!(p.projected_date.as_deref(), Some("2024-12-01"));
    }

    #[test]
    fn positive_return_reaches_faster_than_zero() {
        let goal = sample_goal();
        let band = project_band(
            &goal,
            Decimal::new(4000, 0),
            Decimal::new(1000, 0),
            ScenarioRates::default(),
            "2024-06-01",
        );
        // Higher returns never take longer than lower returns.
        let pess = band.pessimistic.months_to_goal.unwrap();
        let exp = band.expected.months_to_goal.unwrap();
        let opt = band.optimistic.months_to_goal.unwrap();
        assert!(opt <= exp);
        assert!(exp <= pess);
    }

    #[test]
    fn unreachable_goal_reports_not_reached() {
        let goal = sample_goal(); // 10_000 target
        // No contribution and negative growth: never reached.
        let p = project_with_return(
            &goal,
            Decimal::new(100, 0),
            Decimal::ZERO,
            Decimal::new(-5, 2), // -5% annual
            "2024-06-01",
            120,
        );
        assert!(!p.reached);
        assert_eq!(p.months_to_goal, None);
        assert!(p.projected_date.is_none());
    }

    #[test]
    fn already_met_goal_reached_at_zero_months() {
        let goal = sample_goal();
        let p = project_with_return(
            &goal,
            Decimal::new(12000, 0),
            Decimal::new(100, 0),
            Decimal::new(5, 2),
            "2024-06-01",
            MAX_PROJECTION_MONTHS,
        );
        assert!(p.reached);
        assert_eq!(p.months_to_goal, Some(0));
    }

    // ── Item 2: inflation ───────────────────────────────────────────────────

    #[test]
    fn inflation_raises_nominal_target() {
        let goal = sample_goal(); // 10_000
        let adj = inflation_adjusted_target(&goal, Decimal::new(3, 2), 120); // 3%, 10y
        assert_eq!(adj.real_target, Decimal::new(10000, 0));
        assert!(adj.nominal_target > adj.real_target);
        // ~10y of 3% inflation ≈ 1.34x.
        assert!(adj.nominal_target > Decimal::new(13000, 0));
        assert!(adj.nominal_target < Decimal::new(14000, 0));
    }

    #[test]
    fn zero_inflation_is_identity() {
        assert_eq!(
            inflate_target(Decimal::new(5000, 0), Decimal::ZERO, 240),
            Decimal::new(5000, 0)
        );
    }

    #[test]
    fn inflation_makes_goal_take_longer() {
        let goal = sample_goal();
        let no_infl = project_with_return(
            &goal,
            Decimal::new(4000, 0),
            Decimal::new(500, 0),
            Decimal::new(5, 2),
            "2024-06-01",
            MAX_PROJECTION_MONTHS,
        );
        let with_infl = project_with_inflation(
            &goal,
            Decimal::new(4000, 0),
            Decimal::new(500, 0),
            Decimal::new(5, 2),
            Decimal::new(4, 2), // 4% inflation chasing the target up
            "2024-06-01",
            MAX_PROJECTION_MONTHS,
        );
        assert!(with_infl.months_to_goal.unwrap() >= no_infl.months_to_goal.unwrap());
    }

    // ── Item 3: variable contribution schedules ─────────────────────────────

    #[test]
    fn schedule_flat_matches_scenario() {
        let goal = sample_goal();
        let sched = ContributionSchedule::flat(Decimal::new(1000, 0));
        let s = project_schedule(
            &goal,
            Decimal::new(4000, 0),
            &sched,
            Decimal::ZERO,
            "2024-06-01",
            MAX_PROJECTION_MONTHS,
        );
        assert_eq!(s.months_to_goal, Some(6));
        assert_eq!(s.total_contributed, Decimal::new(6000, 0));
    }

    #[test]
    fn step_up_accelerates_goal() {
        let goal = sample_goal();
        let sched = ContributionSchedule {
            base_monthly: Decimal::new(100, 0),
            steps: vec![ContributionStep {
                effective_month: 3,
                monthly_amount: Decimal::new(2000, 0),
            }],
            lump_sums: vec![],
        };
        assert_eq!(sched.contribution_at_month(1), Decimal::new(100, 0));
        assert_eq!(sched.contribution_at_month(3), Decimal::new(2000, 0));
        let s = project_schedule(
            &goal,
            Decimal::new(4000, 0),
            &sched,
            Decimal::ZERO,
            "2024-06-01",
            MAX_PROJECTION_MONTHS,
        );
        // m1: 4100, m2: 4200, m3: 6200, m4: 8200, m5: 10200 -> reached month 5
        assert_eq!(s.months_to_goal, Some(5));
    }

    #[test]
    fn lump_sum_completes_goal() {
        let goal = sample_goal();
        let sched = ContributionSchedule {
            base_monthly: Decimal::new(100, 0),
            steps: vec![],
            lump_sums: vec![LumpSum {
                month: 2,
                amount: Decimal::new(6000, 0),
            }],
        };
        let s = project_schedule(
            &goal,
            Decimal::new(4000, 0),
            &sched,
            Decimal::ZERO,
            "2024-06-01",
            MAX_PROJECTION_MONTHS,
        );
        // m1: 4100, m2: 4200 + 6000 = 10200 -> reached month 2
        assert_eq!(s.months_to_goal, Some(2));
    }

    // ── Item 4: allocation ──────────────────────────────────────────────────

    fn alloc_inputs() -> Vec<AllocationInput> {
        vec![
            AllocationInput {
                goal_id: "a".into(),
                remaining: Decimal::new(6000, 0),
                required_contribution: Some(Decimal::new(500, 0)),
                target_date: Some("2025-06-01".into()),
                priority: 1,
            },
            AllocationInput {
                goal_id: "b".into(),
                remaining: Decimal::new(3000, 0),
                required_contribution: Some(Decimal::new(300, 0)),
                target_date: Some("2024-12-01".into()),
                priority: 5,
            },
        ]
    }

    #[test]
    fn priority_order_funds_high_priority_first() {
        let inputs = alloc_inputs();
        // Budget only covers b (300) fully + 200 to a.
        let out = allocate_budget(
            &inputs,
            Decimal::new(500, 0),
            AllocationStrategy::PriorityOrder,
        );
        let a = out.iter().find(|x| x.goal_id == "a").unwrap();
        let b = out.iter().find(|x| x.goal_id == "b").unwrap();
        assert_eq!(b.allocated, Decimal::new(300, 0));
        assert!(b.meets_required);
        assert_eq!(a.allocated, Decimal::new(200, 0));
        assert!(!a.meets_required);
        assert_eq!(a.shortfall, Decimal::new(300, 0));
    }

    #[test]
    fn deadline_first_funds_earliest_target() {
        let inputs = alloc_inputs();
        // b has earlier deadline (2024-12) so it gets funded first.
        let out = allocate_budget(
            &inputs,
            Decimal::new(300, 0),
            AllocationStrategy::DeadlineFirst,
        );
        let b = out.iter().find(|x| x.goal_id == "b").unwrap();
        let a = out.iter().find(|x| x.goal_id == "a").unwrap();
        assert_eq!(b.allocated, Decimal::new(300, 0));
        assert_eq!(a.allocated, Decimal::ZERO);
    }

    #[test]
    fn proportional_splits_by_need() {
        let inputs = alloc_inputs(); // needs 500 and 300, total 800
        let out = allocate_budget(
            &inputs,
            Decimal::new(800, 0),
            AllocationStrategy::Proportional,
        );
        let a = out.iter().find(|x| x.goal_id == "a").unwrap();
        let b = out.iter().find(|x| x.goal_id == "b").unwrap();
        assert_eq!(a.allocated, Decimal::new(500, 0));
        assert_eq!(b.allocated, Decimal::new(300, 0));
    }

    #[test]
    fn equal_split_divides_evenly() {
        let inputs = alloc_inputs();
        let out = allocate_budget(
            &inputs,
            Decimal::new(400, 0),
            AllocationStrategy::EqualSplit,
        );
        for g in &out {
            assert_eq!(g.allocated, Decimal::new(200, 0));
        }
    }

    #[test]
    fn allocation_surplus_meets_all() {
        let inputs = alloc_inputs();
        let out = allocate_budget(
            &inputs,
            Decimal::new(2000, 0),
            AllocationStrategy::PriorityOrder,
        );
        assert!(out.iter().all(|g| g.meets_required));
        assert!(out.iter().all(|g| g.shortfall.is_zero()));
    }

    // ── Item 5: history estimation ──────────────────────────────────────────

    #[test]
    fn history_estimates_average_monthly_delta() {
        let points = vec![
            ("2024-01-01".to_string(), Decimal::new(1000, 0)),
            ("2024-07-01".to_string(), Decimal::new(4000, 0)),
        ];
        // 3000 over 6 months = 500/month.
        assert_eq!(
            estimate_monthly_rate_from_history(&points),
            Some(Decimal::new(500, 0))
        );
    }

    #[test]
    fn history_needs_two_points() {
        let one = vec![("2024-01-01".to_string(), Decimal::new(1000, 0))];
        assert!(estimate_monthly_rate_from_history(&one).is_none());
        assert!(estimate_monthly_rate_from_history(&[]).is_none());
    }

    #[test]
    fn history_rejects_non_positive_span() {
        let same_month = vec![
            ("2024-01-05".to_string(), Decimal::new(1000, 0)),
            ("2024-01-20".to_string(), Decimal::new(2000, 0)),
        ];
        assert!(estimate_monthly_rate_from_history(&same_month).is_none());
    }
}
