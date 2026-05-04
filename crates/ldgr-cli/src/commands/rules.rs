//! `ldgr rules` — manage import auto-categorization rules.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use uuid::Uuid;

use ldgr_core::import::rules::{ImportRule, MatchType, test_rules};

use crate::session;

/// List all rules.
pub fn run_list(vault_path: &Path) -> Result<()> {
    let vault_dir = session::resolve_vault_dir(Some(vault_path));
    let rules = load_rules(&vault_dir)?;

    if rules.is_empty() {
        eprintln!("No import rules configured.");
        eprintln!(
            "Add one with `ldgr rules add --pattern 'WHOLE FOODS' --account 'Expenses:Food'`."
        );
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec!["Priority", "Pattern", "Match", "Account"]);

    let mut sorted = rules;
    sorted.sort_by_key(|r| std::cmp::Reverse(r.priority));

    for rule in &sorted {
        table.add_row(vec![
            &rule.priority.to_string(),
            &rule.pattern,
            match_type_str(rule.match_type),
            &rule.target_account,
        ]);
    }
    println!("{table}");
    Ok(())
}

/// Add a new rule.
pub fn run_add(
    vault_path: &Path,
    pattern: &str,
    account: &str,
    match_type: &str,
    priority: i64,
) -> Result<()> {
    let vault_dir = session::resolve_vault_dir(Some(vault_path));
    let mut rules = load_rules(&vault_dir)?;

    let mt = parse_match_type(match_type)?;

    let rule = ImportRule {
        id: Uuid::now_v7().to_string(),
        priority,
        pattern: pattern.to_string(),
        match_type: mt,
        target_account: account.to_string(),
    };

    eprintln!(
        "✓ Rule added: '{}' ({}) → {}",
        rule.pattern,
        match_type_str(rule.match_type),
        rule.target_account
    );

    rules.push(rule);
    save_rules(&vault_dir, &rules)?;
    Ok(())
}

/// Delete a rule by pattern.
pub fn run_delete(vault_path: &Path, pattern: &str) -> Result<()> {
    let vault_dir = session::resolve_vault_dir(Some(vault_path));
    let mut rules = load_rules(&vault_dir)?;

    let before = rules.len();
    rules.retain(|r| r.pattern.to_lowercase() != pattern.to_lowercase());

    if rules.len() == before {
        bail!("No rule found with pattern '{pattern}'");
    }

    save_rules(&vault_dir, &rules)?;
    eprintln!("✓ Rule deleted: '{pattern}'");
    Ok(())
}

/// Test which rule would match a given description.
pub fn run_test(vault_path: &Path, description: &str) -> Result<()> {
    let vault_dir = session::resolve_vault_dir(Some(vault_path));
    let rules = load_rules(&vault_dir)?;

    match test_rules(description, &rules) {
        Some(rule) => {
            eprintln!(
                "Match: '{}' ({}) → {}",
                rule.pattern,
                match_type_str(rule.match_type),
                rule.target_account
            );
        }
        None => {
            eprintln!("No matching rule for '{description}'.");
        }
    }
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn load_rules(vault_dir: &Path) -> Result<Vec<ImportRule>> {
    let path = vault_dir.join("rules.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let json = fs::read_to_string(&path).context("failed to read rules.json")?;
    serde_json::from_str(&json).context("invalid rules.json")
}

fn save_rules(vault_dir: &Path, rules: &[ImportRule]) -> Result<()> {
    let path = vault_dir.join("rules.json");
    let json = serde_json::to_string_pretty(rules)?;
    fs::write(&path, json).context("failed to write rules.json")
}

fn parse_match_type(s: &str) -> Result<MatchType> {
    match s.to_lowercase().as_str() {
        "contains" | "c" => Ok(MatchType::Contains),
        "exact" | "e" => Ok(MatchType::Exact),
        "startswith" | "starts_with" | "s" | "starts-with" => Ok(MatchType::StartsWith),
        _ => bail!("Unknown match type: '{s}'. Use: contains, exact, startswith"),
    }
}

fn match_type_str(mt: MatchType) -> &'static str {
    match mt {
        MatchType::Contains => "contains",
        MatchType::Exact => "exact",
        MatchType::StartsWith => "starts-with",
    }
}
