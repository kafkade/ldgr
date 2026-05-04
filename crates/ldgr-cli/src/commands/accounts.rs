//! `ldgr accounts` — account management commands.

use anyhow::{Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};

use ldgr_core::storage::accounts::{
    AccountType, AccountUpdate, ListOptions, NewAccount, create_account, get_account_by_name,
    list_accounts, soft_delete_account, update_account,
};

use crate::db;

/// List all accounts.
pub fn run_list(vault_path: &std::path::Path, flat: bool) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;
    let accounts = list_accounts(&conn, &ListOptions::default())?;

    if accounts.is_empty() {
        eprintln!("No accounts yet. Create one with `ldgr accounts add <name>`.");
        return Ok(());
    }

    if flat {
        for acct in &accounts {
            println!("{}", acct.name);
        }
    } else {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_header(vec!["Account", "Type", "Commodity"]);
        for acct in &accounts {
            table.add_row(vec![
                &acct.name,
                acct.account_type.as_display_str(),
                acct.commodity.as_deref().unwrap_or("—"),
            ]);
        }
        println!("{table}");
    }

    Ok(())
}

/// Create a new account with auto-type detection and parent auto-creation.
pub fn run_add(
    vault_path: &std::path::Path,
    name: &str,
    account_type: Option<&str>,
    commodity: Option<&str>,
) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;

    let acct_type = match account_type {
        Some(t) => parse_account_type(t)?,
        None => detect_account_type(name)?,
    };

    // Auto-create parent accounts
    ensure_parents(&conn, name, acct_type)?;

    let acct = create_account(
        &conn,
        &NewAccount {
            name: name.to_string(),
            account_type: acct_type,
            commodity: commodity.map(String::from),
            parent_id: None,
            note: None,
        },
    )?;

    eprintln!(
        "✓ Created account: {} ({})",
        acct.name,
        acct.account_type.as_display_str()
    );
    Ok(())
}

/// Rename an account, updating all references.
pub fn run_rename(vault_path: &std::path::Path, old_name: &str, new_name: &str) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;

    let acct = get_account_by_name(&conn, old_name)?
        .ok_or_else(|| anyhow::anyhow!("Account '{old_name}' not found"))?;

    let new_type = detect_account_type(new_name).unwrap_or(acct.account_type);

    update_account(
        &conn,
        &acct.id,
        &AccountUpdate {
            name: new_name.to_string(),
            account_type: new_type,
            commodity: acct.commodity.clone(),
            parent_id: acct.parent_id.clone(),
            note: acct.note.clone(),
            expected_version: acct.version,
        },
    )?;

    eprintln!("✓ Renamed: {old_name} → {new_name}");
    Ok(())
}

/// Delete an account (soft delete).
pub fn run_delete(vault_path: &std::path::Path, name: &str) -> Result<()> {
    let conn = db::require_unlocked_db(vault_path)?;

    let acct = get_account_by_name(&conn, name)?
        .ok_or_else(|| anyhow::anyhow!("Account '{name}' not found"))?;

    soft_delete_account(&conn, &acct.id)?;
    eprintln!("✓ Deleted account: {name}");
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Auto-detect account type from the name prefix.
fn detect_account_type(name: &str) -> Result<AccountType> {
    let top = name.split(':').next().unwrap_or(name).to_lowercase();
    match top.as_str() {
        "assets" | "asset" => Ok(AccountType::Asset),
        "liabilities" | "liability" => Ok(AccountType::Liability),
        "income" | "revenue" | "revenues" => Ok(AccountType::Income),
        "expenses" | "expense" => Ok(AccountType::Expense),
        "equity" => Ok(AccountType::Equity),
        _ => bail!(
            "Cannot detect account type from '{name}'.\n\
             Use --type to specify: asset, liability, income, expense, equity.\n\
             Or use a standard prefix like Assets:, Expenses:, Income:, Liabilities:, Equity:"
        ),
    }
}

fn parse_account_type(s: &str) -> Result<AccountType> {
    match s.to_lowercase().as_str() {
        "asset" | "assets" => Ok(AccountType::Asset),
        "liability" | "liabilities" => Ok(AccountType::Liability),
        "income" | "revenue" => Ok(AccountType::Income),
        "expense" | "expenses" => Ok(AccountType::Expense),
        "equity" => Ok(AccountType::Equity),
        _ => bail!("Unknown account type: '{s}'. Use: asset, liability, income, expense, equity"),
    }
}

/// Ensure all parent accounts exist (e.g., for "Assets:Checking:Chase",
/// ensure "Assets" and "Assets:Checking" exist).
fn ensure_parents(
    conn: &rusqlite::Connection,
    name: &str,
    default_type: AccountType,
) -> Result<()> {
    let parts: Vec<&str> = name.split(':').collect();
    if parts.len() <= 1 {
        return Ok(());
    }

    for depth in 1..parts.len() {
        let parent_name = parts[..depth].join(":");
        if get_account_by_name(conn, &parent_name)?.is_none() {
            let parent_type = detect_account_type(&parent_name).unwrap_or(default_type);
            create_account(
                conn,
                &NewAccount {
                    name: parent_name,
                    account_type: parent_type,
                    commodity: None,
                    parent_id: None,
                    note: None,
                },
            )?;
        }
    }

    Ok(())
}

/// Display-friendly account type name.
trait AccountTypeDisplay {
    fn as_display_str(&self) -> &'static str;
}

impl AccountTypeDisplay for AccountType {
    fn as_display_str(&self) -> &'static str {
        match self {
            AccountType::Asset => "Asset",
            AccountType::Liability => "Liability",
            AccountType::Income => "Income",
            AccountType::Expense => "Expense",
            AccountType::Equity => "Equity",
        }
    }
}
