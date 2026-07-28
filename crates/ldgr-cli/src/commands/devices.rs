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

use ldgr_core::sync::framing::open_batch_with_session_key;
use ldgr_core::sync::pairing::{
    Initiation, JoinerSession, PairingCode, deliver_vault_key, initiate_pairing, poll_joiner_hello,
    poll_vault_key, respond_pairing,
};
use ldgr_core::sync::server::{ListBatchesQuery, ServerSyncClient};
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
///
/// When `conn` is supplied the vault identifier comes from the vault itself
/// (ADR-011), which is authoritative — a device that adopted a different vault
/// while pairing has updated it there but not in `sync-config.json`. Callers
/// that hold no unlocked connection fall back to the configured copy.
fn load_server_context(
    vault_dir: &Path,
    conn: Option<&rusqlite::Connection>,
) -> Result<ServerContext> {
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

    let (base_url, configured_vault_id) = match config {
        TransportConfig::Server {
            base_url, vault_id, ..
        } => (base_url, vault_id),
        other => bail!(
            "Device pairing requires the ldgr-server sync provider, but this vault is \
             configured for {}.\nRe-run `ldgr sync setup` and choose ldgr-server.",
            other.provider().as_str()
        ),
    };

    let vault_id = match conn {
        Some(conn) => crate::sync::bridge::resolve_vault_id(conn, vault_dir)?,
        None => configured_vault_id,
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

    let ctx = load_server_context(&vault_dir, Some(&conn))?;
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
    let (conn, vault_key) = crate::db::require_unlocked_db_with_key(vault_path)?;
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let ctx = load_server_context(&vault_dir, Some(&conn))?;
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
/// the vault, and this device adopts the **paired vault's** identifier.
///
/// That adoption is what makes multi-device sync converge. Every device that
/// shares a vault must address the same server vault; before ADR-011 they only
/// agreed by accident, because each independently hashed the same default vault
/// directory path. A joining device now has its own random identifier, so
/// without adopting the paired one it would sync into a second, empty vault and
/// never see the other device's data.
pub fn run_join(vault_path: &Path, payload: &str) -> Result<()> {
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));

    let code = PairingCode::decode(payload).map_err(|e| {
        anyhow::anyhow!(
            "that doesn't look like a valid pairing code: {e}\n\
             Copy it exactly from `ldgr devices add` on the other device."
        )
    })?;

    // Open the working store up front, and require it. Two reasons, both load-
    // bearing:
    //
    // - the adoption below has to write to it, and once `create_session` swaps
    //   the session key for the paired device's, this vault's own SQLCipher
    //   store can no longer be opened (its file key is derived from the *local*
    //   vault key), so there is no second chance later;
    // - failing here happens *before* `respond_pairing` consumes the pairing
    //   code, so the user can simply unlock and retry with the same code instead
    //   of having to generate a fresh one on the other device.
    let conn = crate::db::require_unlocked_db(vault_path).context(
        "the vault must be unlocked to join a pairing — run `ldgr unlock` first, \
         then re-run `ldgr devices join`",
    )?;
    let ctx = load_server_context(&vault_dir, Some(&conn))?;
    let rt = runtime()?;

    let (vault_key, paired_vault_id) = rt.block_on(async {
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

        let paired = discover_paired_vault(&ctx, &key).await;

        // Best-effort: register this device so it appears in `devices list`.
        if let Ok(device_id) = crate::sync::bridge::resolve_device_id(&conn, &vault_dir) {
            let info = DeviceInfo {
                device_id: device_id.clone(),
                name: hostname(),
                platform: "cli".to_string(),
                last_sync_at: None,
                vector_clock: ldgr_core::sync::VectorClock::default(),
            };
            if let Ok(bytes) = serde_json::to_vec(&info) {
                let target = paired.as_deref().unwrap_or(&ctx.vault_id);
                let _ = ctx.client.put_device(target, &device_id, &bytes).await;
            }
        }

        Ok::<([u8; 32], Option<String>), anyhow::Error>((key, paired))
    })?;

    // Point this device at the vault it was just paired into, updating both
    // stores together. `sync_state` takes precedence over `sync-config.json`
    // when the vault id is resolved, so writing only the config would leave
    // push/pull silently addressing this device's own (empty) vault while
    // `devices remove` addressed the paired one.
    if let Some(paired) = paired_vault_id.filter(|p| *p != ctx.vault_id) {
        crate::sync::bridge::persist_vault_id(&conn, &paired)?;
        rewrite_configured_vault_id(&vault_dir, &paired)?;
        println!();
        println!("✓ Adopted the paired vault `{paired}`.");
    }

    // Install the received vault key **last**. It replaces the session key with
    // the paired device's, after which the local working store is no longer
    // openable, so nothing below this point may touch the database.
    drop(conn);
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

/// Work out which of the account's vaults the received key belongs to.
///
/// The key itself is the proof: a batch that decrypts under it is a batch from
/// the paired vault. So we try one batch per candidate and adopt the vault that
/// opens — no extra pairing-protocol fields, and no chance of the user picking
/// the wrong vault by hand.
///
/// Falls back to the account's only vault when there is exactly one, and to
/// `None` (keep the current identifier) when the choice is genuinely ambiguous —
/// several vaults, none with a batch to test against yet.
async fn discover_paired_vault(ctx: &ServerContext, vault_key: &[u8; 32]) -> Option<String> {
    let vaults = ctx.client.list_vaults().await.ok()?;

    match vaults.len() {
        0 => return None,
        1 => return Some(vaults[0].id.clone()),
        _ => {}
    }

    for vault in &vaults {
        let query = ListBatchesQuery {
            limit: Some(1),
            ..ListBatchesQuery::default()
        };
        let Ok(metas) = ctx.client.list_remote_batches(&vault.id, &query).await else {
            continue;
        };
        let Some(meta) = metas.first() else { continue };
        let Ok(ciphertext) = ctx
            .client
            .get_batch(&vault.id, &meta.device_id, &meta.batch_id)
            .await
        else {
            continue;
        };
        if open_batch_with_session_key(vault_key, &ciphertext).is_ok() {
            return Some(vault.id.clone());
        }
    }

    println!();
    println!("⚠ This account has several vaults and none could be matched to the");
    println!("  key you just received. Staying on `{}`.", ctx.vault_id);
    println!("  Run `ldgr sync status` to check which vault this device syncs to.");
    None
}

/// Update the `vault_id` recorded in `sync-config.json` so it agrees with the
/// vault's own identifier. Leaves every other field untouched.
fn rewrite_configured_vault_id(vault_dir: &Path, vault_id: &str) -> Result<()> {
    let config_path = vault_dir.join("sync-config.json");
    let json = std::fs::read_to_string(&config_path).context("failed to read sync config")?;
    let mut config: TransportConfig =
        serde_json::from_str(&json).context("failed to parse sync config")?;

    if let TransportConfig::Server { vault_id: id, .. } = &mut config {
        vault_id.clone_into(id);
    }

    let updated =
        serde_json::to_string_pretty(&config).context("failed to serialize sync config")?;
    std::fs::write(&config_path, updated).context("failed to write sync config")
}

/// `ldgr devices remove <id>` — revoke a device.
pub fn run_remove(vault_path: &Path, device_id: &str) -> Result<()> {
    let vault_dir = crate::session::resolve_vault_dir(Some(vault_path));
    let ctx = load_server_context(&vault_dir, None)?;

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
