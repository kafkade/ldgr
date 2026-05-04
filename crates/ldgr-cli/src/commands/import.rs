//! `ldgr import` — import transactions from CSV files.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use ldgr_core::import::csv::parse_csv;
use ldgr_core::import::profile::{CsvProfile, apply_profile};
use ldgr_core::import::rules::{ImportRule, apply_rules};
use ldgr_core::storage::accounts::get_account_by_name;
use ldgr_core::storage::transactions::{
    NewPosting, NewTransaction, TransactionStatus, create_transaction,
};

use crate::db;
use crate::session;

/// Run the `import` command.
pub fn run(vault_path: &Path, csv_path: &str, profile_name: Option<&str>) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;

    // Read CSV file
    let csv_text =
        fs::read_to_string(csv_path).with_context(|| format!("failed to read '{csv_path}'"))?;

    // Load or create profile
    let vault_dir = session::resolve_vault_dir(Some(vault_path));
    let profile = load_profile(&vault_dir, profile_name, csv_path)?;

    // Parse CSV
    let rows = parse_csv(&csv_text, profile.delimiter);

    if rows.is_empty() {
        bail!("CSV file is empty");
    }

    // Apply profile mapping
    let (mut candidates, errors) = apply_profile(&rows, &profile);

    for err in &errors {
        eprintln!("Warning: {err}");
    }

    if candidates.is_empty() {
        bail!("No valid rows found in CSV");
    }

    // Load and apply rules
    let rules = load_rules(&vault_dir);
    if !rules.is_empty() {
        apply_rules(&mut candidates, &rules);
        let matched = candidates
            .iter()
            .filter(|c| c.target_account.is_some())
            .count();
        eprintln!("Rules matched {matched}/{} transactions", candidates.len());
    }

    // Verify source account exists
    if get_account_by_name(&conn, &profile.default_account)?.is_none() {
        bail!(
            "Source account '{}' not found. Create it with `ldgr accounts add {}`.",
            profile.default_account,
            profile.default_account
        );
    }

    // Import candidates
    let mut imported = 0;
    let mut skipped = 0;

    for candidate in &candidates {
        let Some(target) = &candidate.target_account else {
            eprintln!(
                "  Skipped: {} {} {} (no matching rule)",
                candidate.date, candidate.description, candidate.amount
            );
            skipped += 1;
            continue;
        };
        let target = target.clone();

        // Ensure target account exists
        if get_account_by_name(&conn, &target)?.is_none() {
            eprintln!(
                "  Skipped: {} {} (target account '{}' not found)",
                candidate.date, candidate.description, target
            );
            skipped += 1;
            continue;
        }

        // Determine debit/credit postings
        let amount_str = &candidate.amount;
        let neg_amount = if let Some(stripped) = amount_str.strip_prefix('-') {
            stripped.to_string()
        } else {
            format!("-{amount_str}")
        };

        create_transaction(
            &conn,
            &NewTransaction {
                date: candidate.date.clone(),
                status: TransactionStatus::Cleared,
                code: None,
                description: candidate.description.clone(),
                comment: None,
                postings: vec![
                    NewPosting {
                        account_id: profile.default_account.clone(),
                        amount_quantity: Some(candidate.amount.clone()),
                        amount_commodity: None,
                        balance_assertion_quantity: None,
                        balance_assertion_commodity: None,
                    },
                    NewPosting {
                        account_id: target,
                        amount_quantity: Some(neg_amount),
                        amount_commodity: None,
                        balance_assertion_quantity: None,
                        balance_assertion_commodity: None,
                    },
                ],
            },
        )?;
        imported += 1;
    }

    eprintln!("✓ Imported {imported} transactions ({skipped} skipped)");
    Ok(())
}

/// Load a profile by name from `~/.ldgr/profiles/`.
fn load_profile(vault_dir: &Path, name: Option<&str>, csv_path: &str) -> Result<CsvProfile> {
    if let Some(name) = name {
        let profile_path = vault_dir.join("profiles").join(format!("{name}.json"));
        if profile_path.exists() {
            let json = fs::read_to_string(&profile_path)
                .with_context(|| format!("failed to read profile '{name}'"))?;
            return serde_json::from_str(&json)
                .with_context(|| format!("invalid profile '{name}'"));
        }
        bail!(
            "Profile '{name}' not found at {}.\n\
             Create one with `ldgr import {csv_path} --create-profile`.",
            profile_path.display()
        );
    }

    // Default profile: guess from CSV structure
    eprintln!("No profile specified. Using default US bank format (date=0, desc=1, amount=2).");
    eprintln!("Create a reusable profile with `ldgr import {csv_path} --create-profile`.");
    Ok(CsvProfile::default_us_bank("default", "Assets:Checking"))
}

/// Save a profile to `~/.ldgr/profiles/`.
#[allow(dead_code)] // Will be used by --create-profile flow
pub fn save_profile(vault_dir: &Path, profile: &CsvProfile) -> Result<()> {
    let profiles_dir = vault_dir.join("profiles");
    fs::create_dir_all(&profiles_dir)?;

    let path = profiles_dir.join(format!("{}.json", profile.name));
    let json = serde_json::to_string_pretty(profile)?;
    fs::write(&path, json)?;

    eprintln!("✓ Profile saved: {}", path.display());
    Ok(())
}

/// Load rules from `~/.ldgr/rules.json`.
fn load_rules(vault_dir: &Path) -> Vec<ImportRule> {
    let rules_path = vault_dir.join("rules.json");
    if !rules_path.exists() {
        return Vec::new();
    }

    match fs::read_to_string(&rules_path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}
