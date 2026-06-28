//! `ldgr sync` — cross-device sync commands.
//!
//! Orchestrates push/pull workflows using the blob transport layer.
//! Credentials are stored locally and encrypted with the vault key.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use ldgr_core::sync::server::{ServerSyncClient, ServerSyncError};
use ldgr_core::sync::transport::{TransportConfig, device_path};

use crate::sync::dropbox::DropboxTransport;
use crate::sync::server::{ReqwestSender, ServerTransport};
use crate::sync::webdav::WebDavTransport;
use crate::sync::{BlobTransport, RetryTransport};

/// File name for the SRP session token (a bearer secret), stored alongside the
/// non-secret `sync-config.json` — mirrors how the Dropbox `access_token` is
/// persisted (never inside `TransportConfig`).
const CREDENTIALS_FILE: &str = "sync-credentials.json";

/// Run `ldgr sync setup` — interactive transport configuration.
pub fn run_setup(vault_path: &Path) -> Result<()> {
    let db = crate::db::require_unlocked_db(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    println!("ldgr sync setup");
    println!("================");
    println!();
    println!("Choose a sync provider:");
    println!("  1. Dropbox");
    println!("  2. WebDAV (Nextcloud, ownCloud, etc.)");
    println!("  3. ldgr-server (self-hosted, end-to-end encrypted)");
    println!();

    print!("Provider [1/2/3]: ");
    io::stdout().flush()?;
    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;

    let config = match choice.trim() {
        "1" => setup_dropbox()?,
        "2" => setup_webdav()?,
        "3" => setup_server(&db, &vault_dir)?,
        _ => bail!("Invalid choice. Please enter 1, 2, or 3."),
    };

    // Save config
    let config_path = vault_dir.join("sync-config.json");
    let json = serde_json::to_string_pretty(&config).context("failed to serialize config")?;
    std::fs::write(&config_path, json).context("failed to write sync config")?;

    println!();
    println!("✓ Sync configured with {}.", config.provider().as_str());
    println!("  Config saved to: {}", config_path.display());
    println!();
    println!("Next steps:");
    println!("  ldgr sync push    — push local changes");
    println!("  ldgr sync pull    — pull remote changes");
    println!("  ldgr sync status  — show sync status");

    Ok(())
}

fn setup_dropbox() -> Result<TransportConfig> {
    println!();
    println!("Dropbox Setup");
    println!("─────────────");
    println!("1. Go to https://www.dropbox.com/developers/apps");
    println!("2. Create an app with 'App folder' access");
    println!("3. Copy your App key");
    println!();

    print!("App key: ");
    io::stdout().flush()?;
    let mut app_key = String::new();
    io::stdin().read_line(&mut app_key)?;
    let app_key = app_key.trim().to_string();

    if app_key.is_empty() {
        bail!("App key cannot be empty.");
    }

    print!("Account email (optional, for reference): ");
    io::stdout().flush()?;
    let mut email = String::new();
    io::stdin().read_line(&mut email)?;
    let account_hint = if email.trim().is_empty() {
        None
    } else {
        Some(email.trim().to_string())
    };

    println!();
    println!("To complete setup, you'll need to authorize ldgr with Dropbox.");
    println!("Run `ldgr sync auth` to start the OAuth flow.");

    Ok(TransportConfig::Dropbox {
        app_key,
        account_hint,
    })
}

fn setup_webdav() -> Result<TransportConfig> {
    println!();
    println!("WebDAV Setup");
    println!("────────────");
    println!("Enter your WebDAV server details.");
    println!();
    println!("Examples:");
    println!("  Nextcloud: https://cloud.example.com/remote.php/dav/files/username/ldgr/");
    println!("  Generic:   https://dav.example.com/path/to/sync/");
    println!();

    print!("WebDAV URL: ");
    io::stdout().flush()?;
    let mut base_url = String::new();
    io::stdin().read_line(&mut base_url)?;
    let base_url = base_url.trim().to_string();

    if base_url.is_empty() {
        bail!("WebDAV URL cannot be empty.");
    }

    print!("Username: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username)?;
    let username = if username.trim().is_empty() {
        None
    } else {
        Some(username.trim().to_string())
    };

    println!();
    println!("Password will be requested when syncing (not stored in config).");
    println!("For persistent auth, use `ldgr sync auth`.");

    Ok(TransportConfig::WebDav { base_url, username })
}

/// Interactive setup for the self-hosted `ldgr-server` transport.
///
/// Performs a one-time SRP-6a login (registering the account first if needed),
/// ensures the vault exists, registers this device, and persists the resulting
/// session token to `sync-credentials.json` (a bearer secret, kept out of the
/// non-secret `TransportConfig`). The password is used only for the handshake
/// and is never stored.
///
/// Two-secret onboarding (2SKD, ADR-008 / issue #180) is a future addition: the
/// only change required here is prompting for the account Secret Key and calling
/// `register_2skd` / `login_2skd` in place of the single-secret calls below.
fn setup_server(conn: &rusqlite::Connection, vault_dir: &Path) -> Result<TransportConfig> {
    println!();
    println!("ldgr-server Setup (self-hosted)");
    println!("───────────────────────────────");
    println!("Sync to your own ldgr-server over an end-to-end encrypted SRP-6a");
    println!("session. Your password never leaves this device and is not stored.");
    println!();

    print!("Server URL (e.g. https://sync.example.com): ");
    io::stdout().flush()?;
    let mut base_url = String::new();
    io::stdin().read_line(&mut base_url)?;
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        bail!("Server URL cannot be empty.");
    }

    print!("Username (email): ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username)?;
    let username = username.trim().to_string();
    if username.is_empty() {
        bail!("Username cannot be empty.");
    }

    let password = rpassword::prompt_password("Password: ").context("failed to read password")?;
    if password.is_empty() {
        bail!("Password cannot be empty.");
    }

    let vault_id = get_vault_id(vault_dir);
    let device_id = crate::sync::bridge::resolve_device_id(conn, vault_dir)?;

    let rt = tokio::runtime::Runtime::new().context("failed to create runtime")?;
    let session_token = rt.block_on(async {
        let sender = ReqwestSender::new(base_url.clone());
        let mut client = ServerSyncClient::new(sender);

        // One-time login (`&mut self`). On 401/404 offer to register, since the
        // account may not exist yet on a fresh self-hosted instance.
        match client.login(&username, password.as_bytes()).await {
            Ok(()) => {}
            Err(ServerSyncError::Http { status, .. }) if status == 401 || status == 404 => {
                println!();
                println!("Login failed — account not found or wrong password.");
                print!("Register a new account with these credentials? [y/N]: ");
                io::stdout().flush()?;
                let mut yn = String::new();
                io::stdin().read_line(&mut yn)?;
                if !matches!(yn.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    bail!("Setup aborted. Verify your credentials and try again.");
                }
                client
                    .register(&username, password.as_bytes())
                    .await
                    .map_err(|e| anyhow::anyhow!("registration failed: {e}"))?;
                client
                    .login(&username, password.as_bytes())
                    .await
                    .map_err(|e| anyhow::anyhow!("login after registration failed: {e}"))?;
            }
            Err(e) => bail!("login failed: {e}"),
        }

        // Ensure the vault exists (idempotent — a 409 means it already does).
        if let Err(e) = client.create_vault(&vault_id).await {
            match e {
                ServerSyncError::Http { status: 409, .. } => {}
                other => bail!("failed to create vault: {other}"),
            }
        }

        // Register this device. The real encrypted device blob is uploaded on
        // `sync push`; this is a best-effort placeholder so the device is known.
        let _ = client.put_device(&vault_id, &device_id, b"{}").await;

        let token = client
            .token()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("server did not return a session token"))?;
        Ok::<String, anyhow::Error>(token)
    })?;

    println!();
    println!("✓ Authenticated to {base_url} and registered device {device_id}.");

    // Persist the session token as a secret, separate from the config — same
    // file/pattern as the Dropbox `access_token`.
    store_session_token(vault_dir, &session_token)?;

    Ok(TransportConfig::Server {
        base_url,
        username: Some(username),
        vault_id,
        device_id,
    })
}

/// Persist the SRP session token into `sync-credentials.json`.
///
/// Read-merge-write so we preserve any other provider's keys already in the
/// file (e.g. a Dropbox `access_token`) instead of clobbering them.
fn store_session_token(vault_dir: &Path, session_token: &str) -> Result<()> {
    let creds_path = vault_dir.join(CREDENTIALS_FILE);
    let mut creds: serde_json::Value = if creds_path.exists() {
        let existing =
            std::fs::read_to_string(&creds_path).context("failed to read sync credentials")?;
        if existing.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&existing).context("failed to parse sync credentials")?
        }
    } else {
        serde_json::json!({})
    };
    creds["session_token"] = serde_json::Value::String(session_token.to_string());
    std::fs::write(
        &creds_path,
        serde_json::to_string_pretty(&creds).context("failed to serialize credentials")?,
    )
    .context("failed to write sync credentials")?;

    // Restrict permissions on Unix — this file holds the bearer session token.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&creds_path, perms)
            .context("failed to set credentials permissions")?;
    }

    Ok(())
}

/// Run `ldgr sync push` — push local changes to remote.
///
/// Exports the pending `SQLite` outbox as one encrypted batch via the core
/// pipeline, uploads it through the configured transport, then marks those
/// events synced. Replaces the old `sync-outbox/*.enc` file model.
pub fn run_push(vault_path: &Path) -> Result<()> {
    let (conn, key) = crate::db::require_unlocked_db_with_key(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let config = load_config(&vault_dir)?;
    let vault_id = get_vault_id(&vault_dir);
    let device_id = crate::sync::bridge::resolve_device_id(&conn, &vault_dir)?;

    let rt = tokio::runtime::Runtime::new().context("failed to create runtime")?;
    rt.block_on(async {
        let transport = create_transport(&config, &vault_dir)?;

        println!("Pushing changes to {}…", config.provider().as_str());

        let summary = crate::sync::bridge::push_pending(
            &conn,
            transport.as_ref(),
            &vault_id,
            &device_id,
            &key,
        )
        .await?;

        // Best-effort device descriptor update so other devices can list us.
        let device_info = ldgr_core::sync::DeviceInfo {
            device_id: device_id.clone(),
            name: hostname(),
            platform: "cli".to_string(),
            last_sync_at: Some(chrono::Utc::now().to_rfc3339()),
            vector_clock: ldgr_core::sync::VectorClock::default(),
        };
        if let Ok(device_json) = serde_json::to_vec_pretty(&device_info) {
            transport
                .put_blob(&device_path(&vault_id, &device_id), &device_json)
                .await
                .ok();
        }

        if summary.batches_pushed == 0 {
            println!("No pending changes to push.");
        } else {
            println!(
                "✓ Pushed {} event(s) in {} batch(es).",
                summary.events_pushed, summary.batches_pushed
            );
        }

        Ok(())
    })
}

/// Run `ldgr sync pull` — pull remote changes and apply them.
///
/// Downloads remote batch blobs and feeds each through the core pipeline's
/// `ingest_batch`, materializing accounts/transactions into `SQLite` and
/// persisting any conflicts for review. Replaces the old `sync-inbox/`
/// stage-only behavior.
pub fn run_pull(vault_path: &Path) -> Result<()> {
    let (conn, key) = crate::db::require_unlocked_db_with_key(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let config = load_config(&vault_dir)?;
    let vault_id = get_vault_id(&vault_dir);
    let device_id = crate::sync::bridge::resolve_device_id(&conn, &vault_dir)?;

    let rt = tokio::runtime::Runtime::new().context("failed to create runtime")?;
    rt.block_on(async {
        let transport = create_transport(&config, &vault_dir)?;

        println!("Pulling changes from {}…", config.provider().as_str());

        let summary = crate::sync::bridge::pull_and_apply(
            &conn,
            transport.as_ref(),
            &vault_id,
            &device_id,
            &key,
        )
        .await?;

        if summary.batches_ingested == 0 {
            println!("Already up to date.");
            return Ok(());
        }

        println!(
            "✓ Applied {} change(s) from {} batch(es) ({} skipped).",
            summary.applied, summary.batches_ingested, summary.skipped
        );

        if summary.conflicts > 0 {
            println!();
            println!(
                "⚠ {} conflict(s) need review — run `ldgr sync resolve`.",
                summary.conflicts
            );
        }

        Ok(())
    })
}

/// Run `ldgr sync status` — show sync state.
pub fn run_status(vault_path: &Path) -> Result<()> {
    let conn = crate::db::require_unlocked_db(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let config_path = vault_dir.join("sync-config.json");
    if !config_path.exists() {
        println!("Sync is not configured.");
        println!("Run `ldgr sync setup` to configure a sync provider.");
        return Ok(());
    }

    let config = load_config(&vault_dir)?;
    let device_id = crate::sync::bridge::resolve_device_id(&conn, &vault_dir)?;
    let last_sync = crate::sync::bridge::last_sync_at(&conn)?;
    let pending_push = ldgr_core::storage::sync::pending_event_count(&conn)?;
    let conflicts = ldgr_core::storage::sync::unresolved_conflict_count(&conn)?;

    println!("Sync Status");
    println!("════════════");
    println!("  Provider:    {}", config.provider().as_str());
    println!("  Device ID:   {device_id}");
    println!("  Last sync:   {}", last_sync.as_deref().unwrap_or("never"));
    println!("  Pending push: {pending_push} event(s)");
    println!("  Conflicts:    {conflicts} unresolved");

    if conflicts > 0 {
        println!();
        println!("  ⚠ Run `ldgr sync resolve` to review and resolve conflicts.");
    }

    match &config {
        TransportConfig::Dropbox {
            app_key,
            account_hint,
        } => {
            println!();
            println!("  Dropbox app key: {app_key}");
            if let Some(hint) = account_hint {
                println!("  Account: {hint}");
            }
        }
        TransportConfig::WebDav { base_url, username } => {
            println!();
            println!("  WebDAV URL: {base_url}");
            if let Some(user) = username {
                println!("  Username: {user}");
            }
        }
        TransportConfig::Server {
            base_url,
            username,
            vault_id,
            ..
        } => {
            println!();
            println!("  Server URL: {base_url}");
            if let Some(user) = username {
                println!("  Username:   {user}");
            }
            println!("  Vault ID:   {vault_id}");
        }
    }

    Ok(())
}

/// Run `ldgr sync resolve` — review and resolve pending sync conflicts.
///
/// Conflicts arise when a remote change collides with an un-pushed local change
/// on the same entity. The pipeline keeps the **local** version materialized and
/// records the conflict for explicit review (ADR-003: no silent last-write-wins).
///
/// This minimal resolver lists each unresolved conflict and lets the user keep
/// the local version (marking it resolved). Re-materializing the **remote**
/// version is a scoped follow-up — it needs richer conflict metadata than is
/// stored today — so for now choosing "remote" is reported as unsupported.
pub fn run_resolve(vault_path: &Path) -> Result<()> {
    let conn = crate::db::require_unlocked_db(vault_path)?;

    let conflicts = ldgr_core::storage::sync::list_unresolved_conflicts(&conn)?;
    if conflicts.is_empty() {
        println!("No unresolved conflicts. 🎉");
        return Ok(());
    }

    println!("{} unresolved conflict(s):", conflicts.len());
    println!();

    let mut resolved = 0usize;
    for c in &conflicts {
        println!("Conflict {}", c.id);
        println!("  Entity:    {} {}", c.entity_type, c.entity_id);
        println!("  Detected:  {}", c.detected_at);
        println!("  Local event:  {}", c.local_event_id);
        println!("  Remote event: {}", c.remote_event_id);
        println!();
        print!("  Keep [l]ocal (current) or [s]kip for now? [l/s]: ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;

        match choice.trim().to_ascii_lowercase().as_str() {
            "l" | "local" => {
                ldgr_core::storage::sync::resolve_conflict(&conn, &c.id, "local")?;
                resolved += 1;
                println!("  ✓ Kept local version.");
            }
            _ => {
                println!("  ─ Skipped (still unresolved).");
            }
        }
        println!();
    }

    println!("Resolved {resolved} conflict(s).");
    let remaining = conflicts.len() - resolved;
    if remaining > 0 {
        println!("{remaining} still need review.");
    }
    println!();
    println!(
        "Note: keeping the *remote* version on a conflict is not yet supported \n\
         from the CLI — that re-materialization is a tracked follow-up."
    );

    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn load_config(vault_dir: &Path) -> Result<TransportConfig> {
    let config_path = vault_dir.join("sync-config.json");
    if !config_path.exists() {
        bail!("Sync is not configured.\nRun `ldgr sync setup` to configure a sync provider.");
    }
    let json = std::fs::read_to_string(&config_path).context("failed to read sync config")?;
    serde_json::from_str(&json).context("failed to parse sync config")
}

fn get_vault_id(vault_dir: &Path) -> String {
    let path_str = vault_dir.to_string_lossy();
    let hash = simple_hash(path_str.as_bytes());
    format!("vault_{hash:016x}")
}

fn simple_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 5381;
    for &b in data {
        hash = hash.wrapping_mul(33).wrapping_add(u64::from(b));
    }
    hash
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn create_transport(config: &TransportConfig, vault_dir: &Path) -> Result<Box<dyn BlobTransport>> {
    let policy = ldgr_core::sync::RetryPolicy::default();

    match config {
        TransportConfig::Dropbox { .. } => {
            // Load access token from local credentials
            let creds_path = vault_dir.join(CREDENTIALS_FILE);
            if !creds_path.exists() {
                bail!(
                    "Dropbox credentials not found.\n\
                     Run `ldgr sync auth` to authenticate with Dropbox."
                );
            }
            let creds_json =
                std::fs::read_to_string(&creds_path).context("failed to read credentials")?;
            let creds: serde_json::Value =
                serde_json::from_str(&creds_json).context("failed to parse credentials")?;
            let token = creds["access_token"]
                .as_str()
                .context("missing access_token in credentials")?
                .to_string();

            let transport = DropboxTransport::new(token, String::new());
            let retry = RetryTransport::new(transport, policy);
            Ok(Box::new(retry))
        }
        TransportConfig::WebDav { base_url, username } => {
            // Prompt for password at runtime
            let user = username.clone().unwrap_or_default();
            let password = if user.is_empty() {
                String::new()
            } else {
                rpassword::prompt_password(format!("WebDAV password for {user}: "))
                    .context("failed to read password")?
            };

            let transport = WebDavTransport::new(base_url.clone(), user, password);
            let retry = RetryTransport::new(transport, policy);
            Ok(Box::new(retry))
        }
        TransportConfig::Server {
            base_url, vault_id, ..
        } => {
            // The SRP session token is a secret kept in sync-credentials.json,
            // written at `sync setup` after login (same pattern as Dropbox).
            let creds_path = vault_dir.join(CREDENTIALS_FILE);
            if !creds_path.exists() {
                bail!(
                    "ldgr-server credentials not found.\n\
                     Run `ldgr sync setup` to authenticate with your server."
                );
            }
            let creds_json =
                std::fs::read_to_string(&creds_path).context("failed to read credentials")?;
            let creds: serde_json::Value =
                serde_json::from_str(&creds_json).context("failed to parse credentials")?;
            let token = creds["session_token"]
                .as_str()
                .context(
                    "missing session_token in credentials — re-run `ldgr sync setup` to log in",
                )?
                .to_string();

            // Token-based: the SRP login already happened at `sync setup`. The
            // password is never stored, so an expired token surfaces as an auth
            // error telling the user to re-run setup.
            let transport = ServerTransport::new(base_url.clone(), token, vault_id.clone());
            let retry = RetryTransport::new(transport, policy);
            Ok(Box::new(retry))
        }
    }
}
