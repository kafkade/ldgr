//! `ldgr goals` — advanced financial goal projections.
//!
//! Goal *persistence* as versioned vault entities is tracked separately
//! (#214). Until that lands, goal definitions live in a JSON file at
//! `~/.ldgr/goals.json`. Current balances are pulled from linked accounts in
//! the vault when available, falling back to a per-goal `current_amount`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use ldgr_core::accounting::reports::compute_balance;
use ldgr_core::goals::{
    AllocationInput, AllocationStrategy, ContributionSchedule, Goal, GoalType, ProjectionBand,
    ScenarioRates, ScheduleProjection, allocate_budget, compute_progress,
    inflation_adjusted_target, project_band, project_schedule, project_with_inflation,
    required_contribution,
};
use ldgr_core::storage::accounts::ListOptions;
use ldgr_core::storage::transactions::list_transactions;

use crate::{convert, db, session};

const GOALS_FILE: &str = "goals.json";

/// On-disk goal definition (superset of the core [`Goal`] model).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GoalEntry {
    id: String,
    name: String,
    #[serde(default = "default_goal_type")]
    goal_type: GoalType,
    target_amount: Decimal,
    #[serde(default)]
    target_date: Option<String>,
    /// Account whose balance is the goal's current amount.
    #[serde(default)]
    linked_account: Option<String>,
    /// Funding priority (higher funded first under the priority strategy).
    #[serde(default)]
    priority: i32,
    /// Baseline recurring monthly contribution.
    #[serde(default)]
    monthly_contribution: Decimal,
    /// Fallback current amount when no linked account balance is available.
    #[serde(default)]
    current_amount: Option<Decimal>,
    /// Annual inflation rate applied to the target (e.g. `0.03`).
    #[serde(default)]
    inflation: Option<Decimal>,
    /// Optional variable contribution schedule (used by `goals plan`).
    #[serde(default)]
    schedule: Option<ContributionSchedule>,
}

fn default_goal_type() -> GoalType {
    GoalType::Custom
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GoalsFile {
    #[serde(default)]
    goals: Vec<GoalEntry>,
}

impl GoalEntry {
    fn to_core_goal(&self) -> Goal {
        Goal {
            id: self.id.clone(),
            name: self.name.clone(),
            goal_type: self.goal_type,
            target_amount: self.target_amount,
            target_date: self.target_date.clone(),
            linked_account: self.linked_account.clone(),
        }
    }
}

fn goals_file_path() -> PathBuf {
    session::default_vault_dir().join(GOALS_FILE)
}

fn load_goals() -> Result<GoalsFile> {
    let path = goals_file_path();
    let json = fs::read_to_string(&path).with_context(|| {
        format!(
            "No goals file at {}.\nCreate one with entries like:\n{}",
            path.display(),
            sample_goals_json()
        )
    })?;
    let file: GoalsFile = serde_json::from_str(&json)
        .with_context(|| format!("malformed goals file at {}", path.display()))?;
    Ok(file)
}

fn sample_goals_json() -> &'static str {
    r#"{
  "goals": [
    {
      "id": "emergency",
      "name": "Emergency Fund",
      "goal_type": "EmergencyFund",
      "target_amount": "15000",
      "target_date": "2026-12-31",
      "linked_account": "Assets:Savings:Emergency",
      "priority": 10,
      "monthly_contribution": "500",
      "current_amount": "4000",
      "inflation": "0.03"
    }
  ]
}"#
}

fn find_goal<'a>(file: &'a GoalsFile, id: &str) -> Result<&'a GoalEntry> {
    file.goals
        .iter()
        .find(|g| g.id == id)
        .ok_or_else(|| anyhow::anyhow!("no goal with id '{id}' in {}", goals_file_path().display()))
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn parse_decimal(label: &str, s: &str) -> Result<Decimal> {
    s.parse::<Decimal>()
        .with_context(|| format!("invalid {label} value: '{s}'"))
}

/// Resolve a goal's current amount: linked account balance, else fallback.
fn resolve_current_amount(vault_path: &Path, entry: &GoalEntry) -> Decimal {
    if let Some(account) = &entry.linked_account
        && let Some(balance) = linked_balance(vault_path, account)
    {
        return balance;
    }
    entry.current_amount.unwrap_or(Decimal::ZERO)
}

/// Sum of a linked account's balance across commodities (locked vault -> None).
fn linked_balance(vault_path: &Path, account: &str) -> Option<Decimal> {
    let conn = db::require_unlocked_db(vault_path).ok()?;
    let store_txns = list_transactions(&conn, &ListOptions::default()).ok()?;
    let txns = convert::to_accounting_txns(&store_txns);
    let report = compute_balance(&txns, Some(account), None, None);
    if report.accounts.is_empty() {
        return None;
    }
    Some(report.totals.values().copied().sum())
}

fn months_horizon(entry: &GoalEntry, now: &str) -> u32 {
    entry
        .target_date
        .as_deref()
        .and_then(|target| months_between(now, target))
        .filter(|m| *m > 0)
        .map_or(120, |m| u32::try_from(m).unwrap_or(120))
}

fn months_between(from: &str, to: &str) -> Option<i32> {
    use chrono::Datelike;
    let d1 = chrono::NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let d2 = chrono::NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    #[allow(clippy::cast_possible_wrap)]
    let months = (d2.year() - d1.year()) * 12 + (d2.month() as i32 - d1.month() as i32);
    Some(months)
}

// ── Subcommand: list ────────────────────────────────────────────────────────────

/// List all defined goals with current progress.
pub fn run_list(vault_path: &Path, output: &str) -> Result<()> {
    let file = load_goals()?;
    if file.goals.is_empty() {
        eprintln!("No goals defined in {}.", goals_file_path().display());
        return Ok(());
    }
    let now = today();

    let rows: Vec<_> = file
        .goals
        .iter()
        .map(|e| {
            let current = resolve_current_amount(vault_path, e);
            let progress =
                compute_progress(&e.to_core_goal(), current, e.monthly_contribution, &now);
            (e, current, progress)
        })
        .collect();

    if output == "json" {
        let entries: Vec<serde_json::Value> = rows
            .iter()
            .map(|(e, current, p)| {
                serde_json::json!({
                    "id": e.id,
                    "name": e.name,
                    "target_amount": e.target_amount.to_string(),
                    "current_amount": current.to_string(),
                    "remaining": p.remaining.to_string(),
                    "percent_complete": p.percent_complete.to_string(),
                    "on_track": p.on_track,
                    "projected_date": p.projected_date,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec![
        "ID",
        "Name",
        "Target",
        "Current",
        "Remaining",
        "%",
        "On track",
        "Projected",
    ]);
    for (e, current, p) in &rows {
        table.add_row(vec![
            e.id.clone(),
            e.name.clone(),
            e.target_amount.to_string(),
            current.to_string(),
            p.remaining.to_string(),
            format!("{:.1}", p.percent_complete),
            if p.on_track {
                "yes".into()
            } else {
                "no".into()
            },
            p.projected_date.clone().unwrap_or_else(|| "—".into()),
        ]);
    }
    println!("{table}");
    Ok(())
}

// ── Subcommand: project ─────────────────────────────────────────────────────────

/// Multi-scenario projection band for a single goal.
#[allow(clippy::too_many_arguments)]
pub fn run_project(
    vault_path: &Path,
    id: &str,
    contribution: Option<&str>,
    pessimistic: Option<&str>,
    expected: Option<&str>,
    optimistic: Option<&str>,
    inflation: Option<&str>,
    output: &str,
) -> Result<()> {
    let file = load_goals()?;
    let entry = find_goal(&file, id)?;
    let goal = entry.to_core_goal();
    let now = today();
    let current = resolve_current_amount(vault_path, entry);

    let contribution = match contribution {
        Some(s) => parse_decimal("contribution", s)?,
        None => entry.monthly_contribution,
    };

    let mut rates = ScenarioRates::default();
    if let Some(s) = pessimistic {
        rates.pessimistic = parse_decimal("pessimistic", s)?;
    }
    if let Some(s) = expected {
        rates.expected = parse_decimal("expected", s)?;
    }
    if let Some(s) = optimistic {
        rates.optimistic = parse_decimal("optimistic", s)?;
    }

    let inflation = match inflation {
        Some(s) => Some(parse_decimal("inflation", s)?),
        None => entry.inflation,
    };

    let band = project_band(&goal, current, contribution, rates, &now);

    // When inflation is supplied, also compute the expected path against the
    // inflated (nominal) target so the user sees the real-vs-nominal gap.
    let inflated = inflation.map(|infl| {
        let months = months_horizon(entry, &now);
        let target = inflation_adjusted_target(&goal, infl, months);
        let proj = project_with_inflation(
            &goal,
            current,
            contribution,
            rates.expected,
            infl,
            &now,
            ldgr_core::goals::MAX_PROJECTION_MONTHS,
        );
        (target, proj)
    });

    if output == "json" {
        let mut obj = serde_json::json!({
            "id": entry.id,
            "name": entry.name,
            "current_amount": current.to_string(),
            "monthly_contribution": contribution.to_string(),
            "band": band,
        });
        if let Some((target, proj)) = &inflated {
            obj["inflation_adjusted"] = serde_json::json!({
                "annual_inflation": target.annual_inflation.to_string(),
                "months": target.months,
                "real_target": target.real_target.to_string(),
                "nominal_target": target.nominal_target.round_dp(2).to_string(),
                "expected_vs_nominal": proj,
            });
        }
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    println!(
        "Goal: {} ({})  current {}  contribution {}/mo",
        entry.name, entry.id, current, contribution
    );
    print_band(&band);

    if let Some((target, proj)) = &inflated {
        println!(
            "\nInflation-adjusted ({}% annual over {} mo):",
            target.annual_inflation * Decimal::from(100),
            target.months
        );
        println!(
            "  real target {}  →  nominal target {}",
            target.real_target,
            target.nominal_target.round_dp(2)
        );
        let date = proj
            .projected_date
            .clone()
            .unwrap_or_else(|| "never".into());
        println!(
            "  expected path vs nominal target: {} ({} mo)",
            date,
            proj.months_to_goal
                .map_or_else(|| "—".into(), |m| m.to_string())
        );
    }
    Ok(())
}

fn print_band(band: &ProjectionBand) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec![
        "Scenario",
        "Annual rate",
        "Reached",
        "Months",
        "Projected date",
        "Final balance",
    ]);
    for (label, s) in [
        ("Pessimistic", &band.pessimistic),
        ("Expected", &band.expected),
        ("Optimistic", &band.optimistic),
    ] {
        table.add_row(vec![
            label.to_string(),
            format!("{}%", s.annual_rate * Decimal::from(100)),
            if s.reached { "yes".into() } else { "no".into() },
            s.months_to_goal
                .map_or_else(|| "—".into(), |m| m.to_string()),
            s.projected_date.clone().unwrap_or_else(|| "—".into()),
            s.final_balance.round_dp(2).to_string(),
        ]);
    }
    println!("{table}");
}

// ── Subcommand: plan ────────────────────────────────────────────────────────────

/// Project a goal under its variable contribution schedule.
pub fn run_plan(vault_path: &Path, id: &str, rate: &str, output: &str) -> Result<()> {
    let file = load_goals()?;
    let entry = find_goal(&file, id)?;
    let goal = entry.to_core_goal();
    let now = today();
    let current = resolve_current_amount(vault_path, entry);
    let annual_rate = parse_decimal("rate", rate)?;

    let schedule = entry
        .schedule
        .clone()
        .unwrap_or_else(|| ContributionSchedule::flat(entry.monthly_contribution));

    let proj = project_schedule(
        &goal,
        current,
        &schedule,
        annual_rate,
        &now,
        ldgr_core::goals::MAX_PROJECTION_MONTHS,
    );

    if output == "json" {
        let obj = serde_json::json!({
            "id": entry.id,
            "name": entry.name,
            "current_amount": current.to_string(),
            "annual_rate": annual_rate.to_string(),
            "schedule": schedule,
            "projection": proj,
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    print_schedule(entry, current, annual_rate, &schedule, &proj);
    Ok(())
}

fn print_schedule(
    entry: &GoalEntry,
    current: Decimal,
    annual_rate: Decimal,
    schedule: &ContributionSchedule,
    proj: &ScheduleProjection,
) {
    println!(
        "Goal: {} ({})  current {}  return {}%/yr",
        entry.name,
        entry.id,
        current,
        annual_rate * Decimal::from(100)
    );
    println!("Base contribution: {}/mo", schedule.base_monthly);
    for step in &schedule.steps {
        println!(
            "  step: month {} → {}/mo",
            step.effective_month, step.monthly_amount
        );
    }
    for lump in &schedule.lump_sums {
        println!("  lump sum: month {} → {}", lump.month, lump.amount);
    }
    let date = proj
        .projected_date
        .clone()
        .unwrap_or_else(|| "never".into());
    println!(
        "\nReached: {}   Months: {}   Date: {}",
        if proj.reached { "yes" } else { "no" },
        proj.months_to_goal
            .map_or_else(|| "—".into(), |m| m.to_string()),
        date
    );
    println!(
        "Total contributed: {}   Final balance: {}",
        proj.total_contributed.round_dp(2),
        proj.final_balance.round_dp(2)
    );
}

// ── Subcommand: allocate ────────────────────────────────────────────────────────

/// Allocate a shared monthly budget across all goals.
pub fn run_allocate(vault_path: &Path, budget: &str, strategy: &str, output: &str) -> Result<()> {
    let file = load_goals()?;
    if file.goals.is_empty() {
        bail!("No goals defined in {}.", goals_file_path().display());
    }
    let now = today();
    let budget = parse_decimal("budget", budget)?;
    let strategy = parse_strategy(strategy)?;

    let inputs: Vec<AllocationInput> = file
        .goals
        .iter()
        .map(|e| {
            let goal = e.to_core_goal();
            let current = resolve_current_amount(vault_path, e);
            let remaining = (e.target_amount - current).max(Decimal::ZERO);
            let required = required_contribution(&goal, current, &now);
            AllocationInput {
                goal_id: e.id.clone(),
                remaining,
                required_contribution: required,
                target_date: e.target_date.clone(),
                priority: e.priority,
            }
        })
        .collect();

    let allocations = allocate_budget(&inputs, budget, strategy);

    if output == "json" {
        let obj = serde_json::json!({
            "budget": budget.to_string(),
            "strategy": strategy,
            "allocations": allocations,
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec!["Goal", "Allocated", "Required", "Meets", "Shortfall"]);
    for a in &allocations {
        table.add_row(vec![
            a.goal_id.clone(),
            a.allocated.round_dp(2).to_string(),
            a.required
                .map_or_else(|| "—".into(), |r| r.round_dp(2).to_string()),
            if a.meets_required {
                "yes".into()
            } else {
                "no".into()
            },
            a.shortfall.round_dp(2).to_string(),
        ]);
    }
    let total: Decimal = allocations.iter().map(|a| a.allocated).sum();
    table.add_row(vec!["────────", "────────", "", "", ""]);
    table.add_row(vec![
        "Total".to_string(),
        total.round_dp(2).to_string(),
        String::new(),
        String::new(),
        String::new(),
    ]);
    println!("{table}");
    Ok(())
}

fn parse_strategy(s: &str) -> Result<AllocationStrategy> {
    match s.to_lowercase().as_str() {
        "priority" => Ok(AllocationStrategy::PriorityOrder),
        "deadline" => Ok(AllocationStrategy::DeadlineFirst),
        "proportional" => Ok(AllocationStrategy::Proportional),
        "equal" => Ok(AllocationStrategy::EqualSplit),
        other => bail!("unknown strategy '{other}' (use: priority, deadline, proportional, equal)"),
    }
}
