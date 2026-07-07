//! `ldgr devices` — device pairing and management.
//!
//! Wires the core X25519 onboarding crypto and the `ldgr-server` key-exchange
//! relay (`ldgr_core::sync::pairing`) into an operator-facing command group:
//!
//! - `list`   — show devices registered for this vault's account.
//! - `add`    — existing device: display a QR / pairing code and transfer the
//!   vault key to a joining device over the encrypted relay channel.
//! - `join`   — new device: consume a pairing code and receive + unwrap the
//!   vault key.
//! - `remove` — revoke a device.
//!
//! All I/O (QR rendering, prompts, polling, key storage) lives here in the CLI;
//! the core layer stays transport- and I/O-agnostic (ADR-005). Pairing only
//! works against the self-hosted `ldgr-server` transport, which provides the
//! relay endpoints.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use ldgr_core::sync::pairing::{
    Initiation, JoinerSession, PairingCode, deliver_vault_key, initiate_pairing, poll_joiner_hello,
    poll_vault_key, respond_pairing,
};
use ldgr_core::sync::server::ServerSyncClient;
use ldgr_core::sync::transport::{DeviceInfo, TransportConfig};

use crate::sync::server::ReqwestSender;

/// How long to wait for the other device before giving up.
const PAIRING_TIMEOUT: Duration = Duration::from_mins(5);
/// How often to poll the relay while waiting.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Credentials file holding the bearer session token (owner-only on Unix).
const CREDENTIALS_FILE: &str = "sync-credentials.json";

/// An authenticated server client plus the vault/account context needed to drive
/// the device + relay endpoints.
struct ServerContext {
    client: ServerSyncClient<ReqwestSender>,
    vault_id: String,
    base_url: String,
}

/// Build an authenticated [`ServerContext`] from the local sync configuration.
///
/// Device pairing is only available on the self-hosted `ldgr-server` transport,
/// so this fails with a clear message for other providers or when sync has not
/// been set up.
fn load_server_context(vault_dir: &Path) -> Result<ServerContext> {
    let config_path = vault_dir.join("sync-config.json");
    if !config_path.exists() {
        bail!(
            "Sync is not configured.\n\
             Run `ldgr sync setup` and choose the ldgr-server provider first."
        );
    }
    let json = std::fs::read_to_string(&config_path).context("failed to read sync config")?;
    let config: TransportConfig =
        serde_json::from_str(&json).context("failed to parse sync config")?;

    let (base_url, vault_id) = match config {
        TransportConfig::Server {
            base_url, vault_id, ..
        } => (base_url, vault_id),
        other => bail!(
            "Device pairing requires the ldgr-server sync provider, but this vault is \
             configured for {}.\nRe-run `ldgr sync setup` and choose ldgr-server.",
            other.provider().as_str()
        ),
    };

    let creds_path = vault_dir.join(CREDENTIALS_FILE);
    if !creds_path.exists() {
        bail!(
            "ldgr-server credentials not found.\n\
             Run `ldgr sync setup` to authenticate with your server."
        );
    }
    let creds_json =
        std::fs::read_to_string(&creds_path).context("failed to read sync credentials")?;
    let creds: serde_json::Value =
        serde_json::from_str(&creds_json).context("failed to parse sync credentials")?;
    let token = creds["session_token"]
        .as_str()
        .context("missing session_token in credentials — re-run `ldgr sync setup` to log in")?
        .to_string();

    let client = ServerSyncClient::with_token(ReqwestSender::new(base_url.clone()), token);
    Ok(ServerContext {
        client,
        vault_id,
        base_url,
    })
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Runtime::new().context("failed to create async runtime")
}

/// `ldgr devices list` — show devices registered for this account/vault.
pub fn run_list(vault_path: &Path) -> Result<()> {
    let conn = crate::db::require_unlocked_db(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));
    let this_device = crate::sync::bridge::resolve_device_id(&conn, &vault_dir).ok();

    let ctx = load_server_context(&vault_dir)?;
    let rt = runtime()?;
    let devices = rt
        .block_on(ctx.client.list_devices(&ctx.vault_id))
        .map_err(|e| anyhow::anyhow!("failed to list devices: {e}"))?;

    if devices.is_empty() {
        println!("No devices registered for this vault.");
        println!("Run `ldgr devices add` on this device to pair another one.");
        return Ok(());
    }

    println!("Devices for {}", ctx.base_url);
    println!("════════════════════════════════════════════════════════════");
    for d in &devices {
        let marker = if this_device.as_deref() == Some(d.id.as_str()) {
            "  (this device)"
        } else {
            ""
        };
        let name = device_name(&d.encrypted_info);
        match name {
            Some(name) => println!("  {} — {name}", d.id),
            None => println!("  {}", d.id),
        }
        println!("      updated: {}{marker}", d.updated_at);
    }
    Ok(())
}

/// Best-effort decode of the (server-opaque) device descriptor into a name. The
/// self-hosted server stores the descriptor JSON as-is, so a friendly name can
/// often be shown; placeholders (`{}`) and encrypted blobs simply return `None`.
fn device_name(encrypted_info_hex: &str) -> Option<String> {
    let bytes = hex_decode(encrypted_info_hex)?;
    let info: DeviceInfo = serde_json::from_slice(&bytes).ok()?;
    let name = info.name.trim();
    if name.is_empty() {
        None
    } else {
        Some(format!("{name} ({})", info.platform))
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// `ldgr devices add` — pair a new device from this (already-set-up) device.
///
/// Generates an ephemeral X25519 keypair, opens a relay offer, and displays a QR
/// code + copyable pairing token. Once the new device joins, the vault key is
/// encrypted under the shared secret and delivered over the relay.
pub fn run_add(vault_path: &Path) -> Result<()> {
    let (_conn, vault_key) = crate::db::require_unlocked_db_with_key(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let ctx = load_server_context(&vault_dir)?;
    let rt = runtime()?;

    rt.block_on(async {
        let initiation: Initiation = initiate_pairing(&ctx.client, &ctx.base_url)
            .await
            .map_err(|e| anyhow::anyhow!("failed to start pairing: {e}"))?;

        let token = initiation.code.encode();
        let verification = initiation.code.verification_code.clone();
        let offer_id = initiation.code.offer_id.clone();

        println!("Pair a new device");
        println!("═════════════════");
        println!();
        println!("On the new device, run:");
        println!();
        println!("  ldgr devices join {token}");
        println!();
        println!("…or scan this QR code:");
        println!();
        crate::commands::sync::print_qr(&token);
        println!("Verification code: {verification}");
        println!("(Confirm it matches on the other device before trusting the pairing.)");
        println!();
        println!("Waiting for the other device to join… (Ctrl-C to cancel)");

        let hello = wait_for(PAIRING_TIMEOUT, || async {
            poll_joiner_hello(&ctx.client, &offer_id)
                .await
                .map_err(|e| anyhow::anyhow!("relay error while waiting: {e}"))
        })
        .await?;

        deliver_vault_key(&ctx.client, initiation, &hello, &vault_key)
            .await
            .map_err(|e| anyhow::anyhow!("failed to deliver vault key: {e}"))?;

        println!();
        println!("✓ Vault key delivered over the encrypted channel.");
        println!("  The new device can now run `ldgr sync pull` to download your data.");
        Ok(())
    })
}

/// `ldgr devices join <code>` — receive the vault key on a new device.
///
/// Consumes the pairing code shown by `ldgr devices add`, derives the shared
/// secret, and waits for the encrypted vault key on the relay. On success the
/// key is installed into the local session so `ldgr sync pull` can materialize
/// the vault.
pub fn run_join(vault_path: &Path, payload: &str) -> Result<()> {
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let code = PairingCode::decode(payload).map_err(|e| {
        anyhow::anyhow!(
            "that doesn't look like a valid pairing code: {e}\n\
             Copy it exactly from `ldgr devices add` on the other device."
        )
    })?;

    let ctx = load_server_context(&vault_dir)?;
    let rt = runtime()?;

    let vault_key = rt.block_on(async {
        let session: JoinerSession = respond_pairing(&ctx.client, &code)
            .await
            .map_err(|e| anyhow::anyhow!("failed to join pairing: {e}"))?;

        println!("Joining device");
        println!("══════════════");
        println!();
        println!("Verification code: {}", session.verification_code);
        println!("(Confirm it matches the code shown on the other device.)");
        println!();
        println!("Waiting for the vault key… (Ctrl-C to cancel)");

        let key = wait_for(PAIRING_TIMEOUT, || async {
            poll_vault_key(&ctx.client, &session)
                .await
                .map_err(|e| anyhow::anyhow!("relay error while waiting: {e}"))
        })
        .await?;

        // Best-effort: register this device so it appears in `devices list`.
        if let Ok(conn) = crate::db::require_unlocked_db(vault_path)
            && let Ok(device_id) = crate::sync::bridge::resolve_device_id(&conn, &vault_dir)
        {
            let info = DeviceInfo {
                device_id: device_id.clone(),
                name: hostname(),
                platform: "cli".to_string(),
                last_sync_at: None,
                vector_clock: ldgr_core::sync::VectorClock::default(),
            };
            if let Ok(bytes) = serde_json::to_vec(&info) {
                let _ = ctx
                    .client
                    .put_device(&ctx.vault_id, &device_id, &bytes)
                    .await;
            }
        }

        Ok::<[u8; 32], anyhow::Error>(key)
    })?;

    // Install the received vault key into the local session so subsequent
    // commands (notably `ldgr sync pull`) can use it.
    crate::session::create_session(
        &vault_dir,
        vault_path,
        &vault_key,
        crate::session::DEFAULT_TIMEOUT_MINUTES,
    )
    .context("failed to store the received vault key")?;

    println!();
    println!("✓ Received and unwrapped the vault key.");
    println!("  Next: run `ldgr sync pull` to download and materialize your data.");
    Ok(())
}

/// `ldgr devices remove <id>` — revoke a device.
pub fn run_remove(vault_path: &Path, device_id: &str) -> Result<()> {
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));
    let ctx = load_server_context(&vault_dir)?;

    print!("Remove device `{device_id}`? This revokes its access. [y/N]: ");
    io::stdout().flush()?;
    let mut yn = String::new();
    io::stdin().read_line(&mut yn)?;
    if !matches!(yn.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Aborted.");
        return Ok(());
    }

    let rt = runtime()?;
    rt.block_on(ctx.client.delete_device(&ctx.vault_id, device_id))
        .map_err(|e| anyhow::anyhow!("failed to remove device: {e}"))?;

    println!("✓ Removed device `{device_id}`.");
    Ok(())
}

/// Poll `op` on a fixed interval until it yields `Some`, or time out.
async fn wait_for<T, F, Fut>(timeout: Duration, mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>>>,
{
    let start = std::time::Instant::now();
    loop {
        if let Some(value) = op().await? {
            return Ok(value);
        }
        if start.elapsed() >= timeout {
            bail!("timed out waiting for the other device. Please try again.");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}
