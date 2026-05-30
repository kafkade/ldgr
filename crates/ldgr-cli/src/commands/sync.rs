//! `ldgr sync` — cross-device sync commands.
//!
//! Orchestrates push/pull workflows using the blob transport layer.
//! Credentials are stored locally and encrypted with the vault key.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use ldgr_core::sync::transport::{
    TransportConfig, batch_path, batches_prefix, device_path, parse_batch_path,
};

use crate::sync::dropbox::DropboxTransport;
use crate::sync::webdav::WebDavTransport;
use crate::sync::{BlobTransport, RetryTransport};

/// Run `ldgr sync setup` — interactive transport configuration.
pub fn run_setup(vault_path: &Path) -> Result<()> {
    let _db = crate::db::require_unlocked_db(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    println!("ldgr sync setup");
    println!("================");
    println!();
    println!("Choose a sync provider:");
    println!("  1. Dropbox");
    println!("  2. WebDAV (Nextcloud, ownCloud, etc.)");
    println!();

    print!("Provider [1/2]: ");
    io::stdout().flush()?;
    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;

    let config = match choice.trim() {
        "1" => setup_dropbox()?,
        "2" => setup_webdav()?,
        _ => bail!("Invalid choice. Please enter 1 or 2."),
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

/// Run `ldgr sync push` — push local changes to remote.
pub fn run_push(vault_path: &Path) -> Result<()> {
    let _db = crate::db::require_unlocked_db(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let config = load_config(&vault_dir)?;

    let rt = tokio::runtime::Runtime::new().context("failed to create runtime")?;
    rt.block_on(async {
        let transport = create_transport(&config, &vault_dir)?;

        let vault_id = get_vault_id(&vault_dir);
        let device_id = get_device_id(&vault_dir)?;

        // Ensure directory structure
        let prefix = batches_prefix(&vault_id);
        transport.ensure_directory(&prefix).await.ok();

        let dev_batches = ldgr_core::sync::transport::device_batches_prefix(&vault_id, &device_id);
        transport.ensure_directory(&dev_batches).await.ok();

        // Load checkpoint
        let checkpoint = load_checkpoint(&vault_dir);

        println!("Pushing changes to {}…", config.provider().as_str());

        // Check for local event batches to push
        let outbox_dir = vault_dir.join("sync-outbox");
        if !outbox_dir.exists() {
            println!("No pending changes to push.");
            return Ok(());
        }

        let mut pushed = 0u32;
        for entry in std::fs::read_dir(&outbox_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("enc"))
            {
                continue;
            }

            let batch_id = name.trim_end_matches(".enc");
            if checkpoint.synced_batch_ids.contains(&batch_id.to_string()) {
                continue;
            }

            let data = std::fs::read(entry.path())?;
            let blob_path = batch_path(&vault_id, &device_id, batch_id);

            match transport.put_blob(&blob_path, &data).await {
                Ok(_) => {
                    pushed += 1;
                    println!("  ✓ Pushed batch {batch_id}");
                }
                Err(e) if e.kind == ldgr_core::sync::TransportErrorKind::Conflict => {
                    println!("  ─ Batch {batch_id} already exists (skipping)");
                }
                Err(e) => {
                    eprintln!("  ✗ Failed to push {batch_id}: {e}");
                    return Err(e.into());
                }
            }
        }

        // Update device info
        let device_info = ldgr_core::sync::DeviceInfo {
            device_id: device_id.clone(),
            name: hostname(),
            platform: "cli".to_string(),
            last_sync_at: Some(chrono::Utc::now().to_rfc3339()),
            vector_clock: checkpoint.vector_clock.clone(),
        };
        let device_json = serde_json::to_vec_pretty(&device_info)?;
        let dev_path = device_path(&vault_id, &device_id);
        transport.put_blob(&dev_path, &device_json).await.ok(); // Best-effort

        if pushed == 0 {
            println!("No new changes to push.");
        } else {
            println!("✓ Pushed {pushed} batch(es).");
        }

        Ok(())
    })
}

/// Run `ldgr sync pull` — pull remote changes.
pub fn run_pull(vault_path: &Path) -> Result<()> {
    let _db = crate::db::require_unlocked_db(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let config = load_config(&vault_dir)?;

    let rt = tokio::runtime::Runtime::new().context("failed to create runtime")?;
    rt.block_on(async {
        let transport = create_transport(&config, &vault_dir)?;

        let vault_id = get_vault_id(&vault_dir);
        let device_id = get_device_id(&vault_dir)?;
        let checkpoint = load_checkpoint(&vault_dir);

        println!("Pulling changes from {}…", config.provider().as_str());

        // List all remote batches
        let prefix = batches_prefix(&vault_id);
        let mut all_entries = Vec::new();
        let mut cursor = None;
        loop {
            let result = transport.list_blobs(&prefix, cursor.as_deref()).await?;
            all_entries.extend(result.entries);
            if !result.has_more {
                break;
            }
            cursor = result.cursor;
        }

        // Filter to batches we haven't seen
        let mut new_batches = Vec::new();
        for entry in &all_entries {
            if let Some(batch_ref) = parse_batch_path(&entry.path) {
                // Skip our own batches
                if batch_ref.device_id == device_id {
                    continue;
                }
                // Skip already-synced batches
                if checkpoint.synced_batch_ids.contains(&batch_ref.batch_id) {
                    continue;
                }
                new_batches.push((batch_ref, entry.clone()));
            }
        }

        if new_batches.is_empty() {
            println!("Already up to date.");
            return Ok(());
        }

        println!("Found {} new batch(es) to pull.", new_batches.len());

        // Download and save to inbox
        let inbox_dir = vault_dir.join("sync-inbox");
        std::fs::create_dir_all(&inbox_dir)?;

        let mut pulled = 0u32;
        for (batch_ref, _entry) in &new_batches {
            let blob_path = batch_path(&vault_id, &batch_ref.device_id, &batch_ref.batch_id);
            match transport.get_blob(&blob_path).await {
                Ok(data) => {
                    let local_path = inbox_dir.join(format!(
                        "{}_{}.enc",
                        batch_ref.device_id, batch_ref.batch_id
                    ));
                    std::fs::write(&local_path, &data)?;
                    pulled += 1;
                    println!(
                        "  ✓ Pulled batch {} from {}",
                        batch_ref.batch_id, batch_ref.device_id
                    );
                }
                Err(e) => {
                    eprintln!("  ✗ Failed to pull {}: {e}", batch_ref.batch_id);
                }
            }
        }

        println!("✓ Pulled {pulled} batch(es).");
        println!();
        println!("Pulled batches saved to sync inbox for merge.");
        println!("Run event merge/conflict resolution to apply changes.");

        Ok(())
    })
}

/// Run `ldgr sync status` — show sync state.
pub fn run_status(vault_path: &Path) -> Result<()> {
    let _db = crate::db::require_unlocked_db(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let config_path = vault_dir.join("sync-config.json");
    if !config_path.exists() {
        println!("Sync is not configured.");
        println!("Run `ldgr sync setup` to configure a sync provider.");
        return Ok(());
    }

    let config = load_config(&vault_dir)?;
    let checkpoint = load_checkpoint(&vault_dir);
    let device_id = get_device_id(&vault_dir)?;

    println!("Sync Status");
    println!("════════════");
    println!("  Provider:    {}", config.provider().as_str());
    println!("  Device ID:   {device_id}");
    println!(
        "  Last sync:   {}",
        checkpoint.last_sync_at.as_deref().unwrap_or("never")
    );
    println!("  Synced batches: {}", checkpoint.synced_batch_ids.len());

    // Count outbox
    let outbox_dir = vault_dir.join("sync-outbox");
    let outbox_count = if outbox_dir.exists() {
        std::fs::read_dir(&outbox_dir)?
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".enc"))
            .count()
    } else {
        0
    };

    // Count inbox
    let inbox_dir = vault_dir.join("sync-inbox");
    let inbox_count = if inbox_dir.exists() {
        std::fs::read_dir(&inbox_dir)?
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".enc"))
            .count()
    } else {
        0
    };

    println!("  Pending push: {outbox_count} batch(es)");
    println!("  Pending pull: {inbox_count} batch(es)");

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
    }

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

fn load_checkpoint(vault_dir: &Path) -> ldgr_core::sync::SyncCheckpoint {
    let path = vault_dir.join("sync-checkpoint.json");
    if path.exists() {
        let json = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&json).unwrap_or_default()
    } else {
        ldgr_core::sync::SyncCheckpoint::default()
    }
}

fn get_vault_id(vault_dir: &Path) -> String {
    let path_str = vault_dir.to_string_lossy();
    let hash = simple_hash(path_str.as_bytes());
    format!("vault_{hash:016x}")
}

fn get_device_id(vault_dir: &Path) -> Result<String> {
    let id_path = vault_dir.join("device-id");
    if id_path.exists() {
        let id = std::fs::read_to_string(&id_path)
            .context("failed to read device ID")?
            .trim()
            .to_string();
        if !id.is_empty() {
            return Ok(id);
        }
    }

    let id = uuid::Uuid::now_v7().to_string();
    std::fs::write(&id_path, &id).context("failed to write device ID")?;
    Ok(id)
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
            let creds_path = vault_dir.join("sync-credentials.json");
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
    }
}
