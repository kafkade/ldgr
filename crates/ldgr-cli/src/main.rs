use clap::Parser;

/// ldgr — Zero-knowledge bookkeeping
#[derive(Parser)]
#[command(name = "ldgr", version, about = "Zero-knowledge bookkeeping")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Create a new vault
    Init,
    /// Unlock the vault
    Unlock,
    /// Lock the vault
    Lock,
    /// Show vault status
    Status,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init) => println!("ldgr init — not yet implemented"),
        Some(Commands::Unlock) => println!("ldgr unlock — not yet implemented"),
        Some(Commands::Lock) => println!("ldgr lock — not yet implemented"),
        Some(Commands::Status) => println!("ldgr status — not yet implemented"),
        None => {
            println!("ldgr — Zero-knowledge bookkeeping");
            println!("Run `ldgr --help` for usage.");
        }
    }
}
