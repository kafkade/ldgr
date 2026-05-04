use std::path::PathBuf;
use std::process;

use clap::Parser;

mod commands;
mod db;
mod session;

/// ldgr — Zero-knowledge bookkeeping
#[derive(Parser)]
#[command(name = "ldgr", version, about = "Zero-knowledge bookkeeping")]
struct Cli {
    /// Path to the vault file
    #[arg(long, global = true, value_name = "PATH")]
    vault: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Create a new vault
    Init,
    /// Unlock the vault with your master password
    Unlock {
        /// Session timeout in minutes
        #[arg(long, default_value_t = session::DEFAULT_TIMEOUT_MINUTES)]
        timeout: i64,
    },
    /// Lock the vault (clear the session)
    Lock,
    /// Show vault status
    Status,

    /// Manage accounts
    Accounts {
        #[command(subcommand)]
        action: Option<AccountAction>,
        /// List accounts as plain names (one per line)
        #[arg(long)]
        flat: bool,
    },

    /// Add a new transaction
    Add {
        /// Transaction date (YYYY-MM-DD). Omit for interactive mode.
        #[arg(long)]
        date: Option<String>,
        /// Transaction description
        #[arg(long, alias = "desc")]
        description: Option<String>,
        /// Transaction status: unmarked, pending, cleared
        #[arg(long, default_value = "unmarked")]
        status: String,
        /// Posting in format 'Account  Amount Commodity' (repeat for each posting)
        #[arg(long = "posting", num_args = 1)]
        postings: Vec<String>,
    },

    /// Delete a transaction (soft delete)
    Delete {
        /// Transaction ID
        id: String,
        /// Skip confirmation prompt
        #[arg(long, short)]
        force: bool,
    },

    /// Import transactions from a CSV file
    Import {
        /// Path to the CSV file
        file: String,
        /// Profile name (from ~/.ldgr/profiles/)
        #[arg(long, short)]
        profile: Option<String>,
    },

    /// Manage import auto-categorization rules
    Rules {
        #[command(subcommand)]
        action: Option<RulesAction>,
    },

    /// Show account balances
    Balance {
        /// Filter by account name (substring match)
        account: Option<String>,
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        begin: Option<String>,
        /// End date (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
        /// Show flat account names (no hierarchy indentation)
        #[arg(long)]
        flat: bool,
        /// Output format: table, json, csv
        #[arg(long, short, default_value = "table")]
        output: String,
    },

    /// Show transaction register with running balance
    Register {
        /// Filter by account name (substring match)
        account: Option<String>,
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        begin: Option<String>,
        /// End date (YYYY-MM-DD)
        #[arg(long)]
        end: Option<String>,
        /// Output format: table, json, csv
        #[arg(long, short, default_value = "table")]
        output: String,
    },
}

#[derive(clap::Subcommand)]
enum AccountAction {
    /// Create a new account
    Add {
        /// Account name (e.g., Assets:Checking:Chase)
        name: String,
        /// Account type: asset, liability, income, expense, equity
        #[arg(long, short = 't')]
        r#type: Option<String>,
        /// Default commodity (e.g., USD)
        #[arg(long, short)]
        commodity: Option<String>,
    },
    /// Rename an account
    Rename {
        /// Current account name
        old: String,
        /// New account name
        new: String,
    },
    /// Delete an account (soft delete)
    Delete {
        /// Account name
        name: String,
    },
}

#[derive(clap::Subcommand)]
enum RulesAction {
    /// Add a new rule
    Add {
        /// Pattern to match against transaction descriptions
        #[arg(long, short)]
        pattern: String,
        /// Target account to assign
        #[arg(long, short)]
        account: String,
        /// Match type: contains, exact, startswith
        #[arg(long, short, default_value = "contains")]
        r#match: String,
        /// Rule priority (higher = checked first)
        #[arg(long, default_value_t = 0)]
        priority: i64,
    },
    /// Delete a rule by pattern
    Delete {
        /// Pattern of the rule to delete
        pattern: String,
    },
    /// Test which rule matches a description
    Test {
        /// Description to test against rules
        description: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let vault_path = session::resolve_vault_path(cli.vault.as_deref());

    let result = match cli.command {
        Some(Commands::Init) => commands::init::run(&vault_path),
        Some(Commands::Unlock { timeout }) => commands::unlock::run(&vault_path, timeout),
        Some(Commands::Lock) => commands::lock::run(&vault_path),
        Some(Commands::Status) => commands::status::run(&vault_path),
        Some(Commands::Accounts { action, flat }) => match action {
            Some(AccountAction::Add {
                name,
                r#type,
                commodity,
            }) => commands::accounts::run_add(
                &vault_path,
                &name,
                r#type.as_deref(),
                commodity.as_deref(),
            ),
            Some(AccountAction::Rename { old, new }) => {
                commands::accounts::run_rename(&vault_path, &old, &new)
            }
            Some(AccountAction::Delete { name }) => {
                commands::accounts::run_delete(&vault_path, &name)
            }
            None => commands::accounts::run_list(&vault_path, flat),
        },
        Some(Commands::Add {
            date,
            description,
            status,
            postings,
        }) => {
            if let (Some(date), Some(desc)) = (date.as_deref(), description.as_deref()) {
                commands::add::run_noninteractive(&vault_path, date, desc, &status, &postings)
            } else {
                commands::add::run_interactive(&vault_path)
            }
        }
        Some(Commands::Delete { id, force }) => commands::delete::run(&vault_path, &id, force),
        Some(Commands::Import { file, profile }) => {
            commands::import::run(&vault_path, &file, profile.as_deref())
        }
        Some(Commands::Rules { action }) => match action {
            Some(RulesAction::Add {
                pattern,
                account,
                r#match,
                priority,
            }) => commands::rules::run_add(&vault_path, &pattern, &account, &r#match, priority),
            Some(RulesAction::Delete { pattern }) => {
                commands::rules::run_delete(&vault_path, &pattern)
            }
            Some(RulesAction::Test { description }) => {
                commands::rules::run_test(&vault_path, &description)
            }
            None => commands::rules::run_list(&vault_path),
        },
        Some(Commands::Balance {
            account,
            begin,
            end,
            flat,
            output,
        }) => commands::balance::run(
            &vault_path,
            account.as_deref(),
            begin.as_deref(),
            end.as_deref(),
            flat,
            &output,
        ),
        Some(Commands::Register {
            account,
            begin,
            end,
            output,
        }) => commands::register::run(
            &vault_path,
            account.as_deref(),
            begin.as_deref(),
            end.as_deref(),
            &output,
        ),
        None => {
            eprintln!("ldgr — Zero-knowledge bookkeeping");
            eprintln!("Run `ldgr --help` for usage.");
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        process::exit(1);
    }
}
