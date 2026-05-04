use std::path::PathBuf;
use std::process;

use clap::Parser;

mod commands;
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
}

fn main() {
    let cli = Cli::parse();
    let vault_path = session::resolve_vault_path(cli.vault.as_deref());

    let result = match cli.command {
        Some(Commands::Init) => commands::init::run(&vault_path),
        Some(Commands::Unlock { timeout }) => commands::unlock::run(&vault_path, timeout),
        Some(Commands::Lock) => commands::lock::run(&vault_path),
        Some(Commands::Status) => commands::status::run(&vault_path),
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
