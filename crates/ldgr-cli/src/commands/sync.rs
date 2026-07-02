//! `ldgr sync` — cross-device sync commands.
//!
//! Orchestrates push/pull workflows using the blob transport layer.
//! Credentials are stored locally and encrypted with the vault key.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use ldgr_core::crypto::{
    Argon2Params, AuthKey, EmergencyKit, SecretKey, derive_auth_key, derive_master_key, open_vault,
};
use ldgr_core::sync::server::{PROTOCOL_VERSION, ServerInfo, ServerSyncClient, ServerSyncError};
use ldgr_core::sync::transport::{TransportConfig, device_path};
use uuid::Uuid;

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
        "3" => setup_server(&db, vault_path, &vault_dir)?,
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
/// Validates the server URL against `/server/info`, then branches on the
/// server's advertised capabilities: a two-secret (2SKD, ADR-008) server runs
/// the Secret Key + Emergency Kit onboarding; a legacy single-secret server
/// falls back to the plain SRP-6a flow. The master password is used only to
/// derive keys locally and is never stored.
fn setup_server(
    conn: &rusqlite::Connection,
    vault_path: &Path,
    vault_dir: &Path,
) -> Result<TransportConfig> {
    println!();
    println!("ldgr-server Setup (self-hosted)");
    println!("───────────────────────────────");
    println!("Sync to your own ldgr-server over an end-to-end encrypted session.");
    println!("Your password never leaves this device and is not stored.");
    println!();

    print!("Server URL (e.g. https://sync.example.com): ");
    io::stdout().flush()?;
    let mut base_url = String::new();
    io::stdin().read_line(&mut base_url)?;
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        bail!("Server URL cannot be empty.");
    }

    let rt = tokio::runtime::Runtime::new().context("failed to create runtime")?;

    // Validate the URL and discover capabilities before asking for credentials.
    let info = rt
        .block_on(async {
            let sender = ReqwestSender::new(base_url.clone());
            let client = ServerSyncClient::new(sender);
            client.server_info().await
        })
        .map_err(|e| {
            anyhow::anyhow!(
                "could not reach an ldgr-server at {base_url}: {e}\n\
                 Check the URL and that the server is running."
            )
        })?;
    print_server_info(&info);

    if info.two_secret_auth {
        setup_server_2skd(&rt, conn, vault_path, vault_dir, base_url, &info)
    } else {
        println!("This server uses single-secret authentication.");
        println!();
        setup_server_single_secret(&rt, conn, vault_dir, base_url)
    }
}

/// Print the server discovery document and warn on a protocol-version mismatch.
fn print_server_info(info: &ServerInfo) {
    println!();
    println!("✓ Connected to ldgr-server");
    println!("  Name:              {}", info.name);
    println!("  Version:           {}", info.version);
    println!("  Protocol:          {}", info.protocol_version);
    println!("  Registration:      {}", info.registration_policy);
    println!(
        "  Two-secret auth:   {}",
        if info.two_secret_auth { "yes" } else { "no" }
    );
    println!();

    if PROTOCOL_VERSION < info.min_protocol_version || PROTOCOL_VERSION > info.max_protocol_version
    {
        println!(
            "⚠  Protocol mismatch: this client speaks v{PROTOCOL_VERSION}, but the server \
             supports v{}–v{}.",
            info.min_protocol_version, info.max_protocol_version
        );
        println!("   Sync may not work correctly. Consider updating ldgr.");
        println!();
    }
}

/// Two-secret (2SKD, ADR-008) onboarding: sign up (generating a Secret Key +
/// Emergency Kit) or sign in — either on a device that already stores the
/// Secret Key, or a new device where the user supplies it.
fn setup_server_2skd(
    rt: &tokio::runtime::Runtime,
    conn: &rusqlite::Connection,
    vault_path: &Path,
    vault_dir: &Path,
    base_url: String,
    info: &ServerInfo,
) -> Result<TransportConfig> {
    print!("Username (email): ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username)?;
    let username = username.trim().to_string();
    if username.is_empty() {
        bail!("Username cannot be empty.");
    }

    let password =
        rpassword::prompt_password("Master password: ").context("failed to read password")?;
    if password.is_empty() {
        bail!("Password cannot be empty.");
    }

    // MK_auth = HKDF(MK) where MK is the vault master key (ADR-008). Deriving it
    // from the vault also validates that the password matches this vault.
    let mk_auth = derive_server_auth_key(vault_path, password.as_bytes())?;

    let vault_id = get_vault_id(vault_dir);
    let device_id = crate::sync::bridge::resolve_device_id(conn, vault_dir)?;

    // If this device already stored a Secret Key, sign in with password only.
    let existing_secret_key = load_secret_key(vault_dir)?;

    let outcome = rt.block_on(async {
        let sender = ReqwestSender::new(base_url.clone());
        let mut client = ServerSyncClient::new(sender);

        let secret_key_text = if let Some(sk_text) = existing_secret_key {
            let sk = SecretKey::parse(&sk_text)
                .map_err(|e| anyhow::anyhow!("stored Secret Key is invalid: {e}"))?;
            client
                .login_2skd(&username, &mk_auth, &sk)
                .await
                .map_err(|e| anyhow::anyhow!("sign-in failed: {e}"))?;
            println!();
            println!("✓ Signed in with the Secret Key stored on this device.");
            sk_text
        } else {
            prompt_2skd_first_time(&mut client, &username, &mk_auth, &base_url, info).await?
        };

        // Ensure the vault exists (idempotent — a 409 means it already does).
        if let Err(e) = client.create_vault(&vault_id).await {
            match e {
                ServerSyncError::Http { status: 409, .. } => {}
                other => bail!("failed to create vault: {other}"),
            }
        }
        // Best-effort device registration; the encrypted blob is sent on push.
        let _ = client.put_device(&vault_id, &device_id, b"{}").await;

        let token = client
            .token()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("server did not return a session token"))?;
        Ok::<(String, String), anyhow::Error>((token, secret_key_text))
    })?;

    let (session_token, secret_key_text) = outcome;

    println!();
    println!("✓ Authenticated to {base_url} and registered device {device_id}.");

    store_session_token(vault_dir, &session_token)?;
    store_secret_key(vault_dir, &secret_key_text)?;

    Ok(TransportConfig::Server {
        base_url,
        username: Some(username),
        vault_id,
        device_id,
    })
}

/// First-time 2SKD onboarding on this device: choose to create a new account
/// (generating a Secret Key + Emergency Kit) or sign in an existing account by
/// entering its Secret Key. Returns the account Secret Key (canonical text).
async fn prompt_2skd_first_time(
    client: &mut ServerSyncClient<ReqwestSender>,
    username: &str,
    mk_auth: &AuthKey,
    base_url: &str,
    info: &ServerInfo,
) -> Result<String> {
    println!();
    println!("No Secret Key is stored on this device. Choose an option:");
    println!("  1. Create a new account (generates your Secret Key + Emergency Kit)");
    println!("  2. Sign in an existing account (enter its Secret Key)");
    println!();
    print!("Choice [1/2]: ");
    io::stdout().flush()?;
    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;

    match choice.trim() {
        "1" => {
            if info.registration_policy == "admin-only" {
                bail!(
                    "This server does not allow self-registration (policy: admin-only).\n\
                     Ask an administrator to create your account, then choose option 2."
                );
            }
            // Client-generated account id bound into the verifier (ADR-008); the
            // server stores it and returns it at login on other devices.
            let account_id = Uuid::now_v7();
            let secret_key = SecretKey::generate(account_id);
            let secret_key_text = secret_key.encode();

            client
                .register_2skd(username, &account_id, mk_auth, &secret_key)
                .await
                .map_err(|e| anyhow::anyhow!("registration failed: {e}"))?;
            client
                .login_2skd(username, mk_auth, &secret_key)
                .await
                .map_err(|e| anyhow::anyhow!("login after registration failed: {e}"))?;

            render_emergency_kit(base_url, username, &secret_key)?;
            Ok(secret_key_text)
        }
        "2" => {
            let sk_text = rpassword::prompt_password("Account Secret Key (A1-…): ")
                .map_err(|e| anyhow::anyhow!("failed to read Secret Key: {e}"))?;
            let sk_text = sk_text.trim().to_string();
            if sk_text.is_empty() {
                bail!("Secret Key cannot be empty.");
            }
            let sk = SecretKey::parse(&sk_text).map_err(|e| {
                anyhow::anyhow!(
                    "that doesn't look like a valid Secret Key: {e}\n\
                     Copy it exactly from your Emergency Kit (starts with `A1-`)."
                )
            })?;
            client
                .login_2skd(username, mk_auth, &sk)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "sign-in failed: {e}\n\
                     Check the master password and Secret Key are both correct."
                    )
                })?;
            Ok(sk_text)
        }
        other => bail!("Invalid choice `{other}`. Enter 1 or 2."),
    }
}

/// Derive the server auth key (`MK_auth`, ADR-008) from the vault's master
/// password, using the Argon2 salt/params stored in the vault header. Opening
/// the vault first validates the password and yields those parameters.
fn derive_server_auth_key(vault_path: &Path, password: &[u8]) -> Result<AuthKey> {
    let bytes = std::fs::read(vault_path)
        .with_context(|| format!("failed to read vault at {}", vault_path.display()))?;
    let vault = open_vault(&bytes, password)
        .map_err(|_| anyhow::anyhow!("Incorrect master password for this vault."))?;
    let (salt, params) = vault.kdf_params();
    let params: Argon2Params = params.clone();
    let master_key = derive_master_key(password, salt, &params)
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
    derive_auth_key(&master_key).map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))
}

/// Render the Emergency Kit once at sign-up: human-readable details, a scannable
/// terminal QR of the kit payload, and an optional file export.
fn render_emergency_kit(base_url: &str, email: &str, secret_key: &SecretKey) -> Result<()> {
    let kit = EmergencyKit::new(base_url.to_string(), email.to_string(), secret_key);
    let qr_payload = kit
        .to_qr_payload()
        .map_err(|e| anyhow::anyhow!("failed to build Emergency Kit QR: {e}"))?;
    let secret_key_text = secret_key.encode();

    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║                          EMERGENCY KIT                             ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    println!("Save this now — your Secret Key is shown ONCE and is required to");
    println!("sign in on a NEW device (together with your master password).");
    println!("It does NOT unlock your local vault and is never sent to the server.");
    println!();
    println!("  Server:       {base_url}");
    println!("  Account:      {email}");
    println!("  Account hint: {}", secret_key.account_hint());
    println!("  Secret Key:   {secret_key_text}");
    println!();
    print_qr(&qr_payload);
    println!("Scan the QR above with another device, or type the Secret Key.");
    println!();

    print!("Save the Emergency Kit to a file? [y/N]: ");
    io::stdout().flush()?;
    let mut yn = String::new();
    io::stdin().read_line(&mut yn)?;
    if matches!(yn.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        print!("File path [emergency-kit.txt]: ");
        io::stdout().flush()?;
        let mut path = String::new();
        io::stdin().read_line(&mut path)?;
        let path = path.trim();
        let path = if path.is_empty() {
            "emergency-kit.txt"
        } else {
            path
        };
        write_emergency_kit_file(
            Path::new(path),
            base_url,
            email,
            &secret_key_text,
            &qr_payload,
        )?;
        println!("✓ Emergency Kit written to {path} (permissions restricted to you).");
        println!("  Store it somewhere safe and offline, then consider deleting the file.");
    } else {
        println!("Not saved. Make sure you've recorded the Secret Key above.");
    }
    println!();
    Ok(())
}

/// Render a QR code to the terminal using half-block characters. Colors are
/// inverted (light modules filled) so it scans on a typical dark terminal.
fn print_qr(payload: &str) {
    match qrcode::QrCode::new(payload.as_bytes()) {
        Ok(code) => {
            let rendered = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .dark_color(qrcode::render::unicode::Dense1x2::Light)
                .light_color(qrcode::render::unicode::Dense1x2::Dark)
                .quiet_zone(true)
                .build();
            println!("{rendered}");
        }
        Err(e) => {
            println!("(could not render QR code: {e})");
        }
    }
}

/// Write `contents` to `path`, ensuring the file is owner-only (`0600`) on Unix
/// from the moment it is created — so a secret is never briefly group/world
/// readable during first creation (the old write-then-chmod pattern left a small
/// window at the process umask, typically `0644`).
fn write_secret_file(path: &Path, contents: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("failed to open {} for writing", path.display()))?;
        file.write_all(contents.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
        // Tighten perms in case the file already existed with a looser mode
        // (`.mode()` only applies when the file is newly created).
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to restrict {} permissions", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

/// Write the Emergency Kit to a file with owner-only permissions on Unix.
fn write_emergency_kit_file(
    path: &Path,
    base_url: &str,
    email: &str,
    secret_key_text: &str,
    qr_payload: &str,
) -> Result<()> {
    let contents = format!(
        "ldgr Emergency Kit\n\
         ==================\n\n\
         KEEP THIS SAFE. The Secret Key below is required to sign in on a new\n\
         device together with your master password. It does not unlock your\n\
         local vault and is never sent to the server.\n\n\
         Server:       {base_url}\n\
         Account:      {email}\n\
         Secret Key:   {secret_key_text}\n\n\
         QR payload (for import/scan):\n{qr_payload}\n"
    );
    write_secret_file(path, &contents)
}

/// Single-secret SRP-6a onboarding (legacy servers without two-secret auth).
fn setup_server_single_secret(
    rt: &tokio::runtime::Runtime,
    conn: &rusqlite::Connection,
    vault_dir: &Path,
    base_url: String,
) -> Result<TransportConfig> {
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
    // This file holds the bearer session token — write it owner-only.
    write_secret_file(
        &creds_path,
        &serde_json::to_string_pretty(&creds).context("failed to serialize credentials")?,
    )?;

    Ok(())
}

/// Load the account Secret Key from `sync-credentials.json`, if present.
///
/// The Secret Key (ADR-008) is a per-account server-auth secret that lets this
/// device sign in with the master password alone. It never unlocks the vault.
fn load_secret_key(vault_dir: &Path) -> Result<Option<String>> {
    let creds_path = vault_dir.join(CREDENTIALS_FILE);
    if !creds_path.exists() {
        return Ok(None);
    }
    let existing =
        std::fs::read_to_string(&creds_path).context("failed to read sync credentials")?;
    if existing.trim().is_empty() {
        return Ok(None);
    }
    let creds: serde_json::Value =
        serde_json::from_str(&existing).context("failed to parse sync credentials")?;
    Ok(creds
        .get("secret_key")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string))
}

/// Persist the account Secret Key into `sync-credentials.json` (0600 on Unix).
///
/// Read-merge-write to preserve other providers' keys (e.g. `session_token`,
/// a Dropbox `access_token`) already in the file.
fn store_secret_key(vault_dir: &Path, secret_key: &str) -> Result<()> {
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
    creds["secret_key"] = serde_json::Value::String(secret_key.to_string());
    // This file holds the account Secret Key — write it owner-only.
    write_secret_file(
        &creds_path,
        &serde_json::to_string_pretty(&creds).context("failed to serialize credentials")?,
    )?;

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
/// This resolver lists each unresolved conflict and lets the user keep the
/// **local** version (a metadata-only resolution) or the **remote** version
/// (re-materialized locally and re-broadcast so every device converges — see
/// [`ldgr_core::sync::pipeline::resolve_conflict_keep_remote`]).
pub fn run_resolve(vault_path: &Path) -> Result<()> {
    let conn = crate::db::require_unlocked_db(vault_path)?;

    let device_id = ldgr_core::storage::sync::device_id(&conn)?;

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
        println!(
            "  Remote event: {} ({}, v{})",
            c.remote_event_id, c.remote_operation, c.remote_version
        );
        println!();
        print!("  Keep [l]ocal (current), keep [r]emote, or [s]kip for now? [l/r/s]: ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;

        match choice.trim().to_ascii_lowercase().as_str() {
            "l" | "local" => {
                ldgr_core::storage::sync::resolve_conflict(&conn, &c.id, "local")?;
                resolved += 1;
                println!("  ✓ Kept local version.");
            }
            "r" | "remote" => {
                ldgr_core::sync::pipeline::resolve_conflict_keep_remote(&conn, &device_id, &c.id)?;
                resolved += 1;
                println!("  ✓ Kept remote version (re-applied locally and queued for sync).");
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
