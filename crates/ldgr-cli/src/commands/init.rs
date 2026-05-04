//! `ldgr init` — create a new vault.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use ldgr_core::crypto::{Argon2Params, create_vault, encode_recovery_key, serialize_vault};

use crate::session;

/// Run the `init` command.
pub fn run(vault_path: &Path) -> Result<()> {
    if vault_path.exists() {
        bail!(
            "Vault already exists at {}\nUse `ldgr unlock` to open it.",
            vault_path.display()
        );
    }

    // Prompt for password
    let password =
        rpassword::prompt_password("Master password: ").context("failed to read password")?;
    if password.is_empty() {
        bail!("Password cannot be empty.");
    }
    let confirm = rpassword::prompt_password("Confirm password: ")
        .context("failed to read password confirmation")?;
    if password != confirm {
        bail!("Passwords do not match.");
    }

    eprintln!("\nDeriving keys (this may take a moment)...");

    let vault_name = vault_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("ldgr vault");

    let (vault, recovery_key) =
        create_vault(password.as_bytes(), vault_name, &Argon2Params::desktop())
            .context("failed to create vault")?;

    // Serialize and write to disk
    let bytes = serialize_vault(&vault).context("failed to serialize vault")?;

    if let Some(parent) = vault_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    fs::write(vault_path, &bytes)
        .with_context(|| format!("failed to write vault to {}", vault_path.display()))?;

    // Restrict vault file permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(vault_path, perms).ok();
    }

    // Initialize SQLite schema alongside the vault
    let db_path = session::resolve_vault_dir(Some(vault_path)).join("vault.db");
    let conn = rusqlite::Connection::open(&db_path)
        .with_context(|| format!("failed to create database at {}", db_path.display()))?;
    ldgr_core::storage::schema::initialize(&conn)
        .context("failed to initialize database schema")?;

    // Display recovery key
    let encoded = encode_recovery_key(&recovery_key);
    eprintln!();
    eprintln!("╔══════════════════════════════════════════════════════════════════╗");
    eprintln!("║                      RECOVERY KEY                              ║");
    eprintln!("║                                                                ║");
    eprintln!("║  Write this down and store it in a safe place.                 ║");
    eprintln!("║  If you lose your password, this is the ONLY way to recover    ║");
    eprintln!("║  your data.                                                    ║");
    eprintln!("║                                                                ║");
    eprintln!("║  {encoded:<62}║");
    eprintln!("║                                                                ║");
    eprintln!("║  Lost password + lost recovery key = UNRECOVERABLE DATA        ║");
    eprintln!("╚══════════════════════════════════════════════════════════════════╝");
    eprintln!();
    eprintln!("✓ Vault created at {}", vault_path.display());
    eprintln!("✓ Database initialized at {}", db_path.display());
    eprintln!();
    eprintln!("Run `ldgr unlock` to start using your vault.");

    Ok(())
}
