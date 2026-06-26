//! Vault binary file format — create, open, save, validate, recover.
//!
//! The vault format is a binary container holding:
//! - A header with KDF parameters, wrapped vault key, and encrypted metadata
//! - A body with a sequence of individually encrypted items
//!
//! All key material in the header is encrypted: the vault key is wrapped by
//! the master encryption key (MEK, derived from the password) and by the
//! recovery key. Metadata (vault name, creation timestamp) is encrypted with
//! the vault key via envelope encryption.
//!
//! **This module performs no I/O.** Callers provide raw bytes and receive raw
//! bytes. File read/write is the responsibility of platform-specific code.
//!
//! # Binary Layout (v1)
//!
//! ```text
//! HEADER (fixed, 51 bytes):
//!   [0..4]    Magic: b"LDGR"
//!   [4..6]    Format version: u16 LE (= 1)
//!   [6..7]    KDF version: u8 (= 1 → Argon2id v0x13)
//!   [7..39]   Salt: [u8; 32]
//!   [39..43]  Argon2 memory cost (KiB): u32 LE
//!   [43..47]  Argon2 iterations: u32 LE
//!   [47..51]  Argon2 parallelism: u32 LE
//!
//! HEADER (variable — binary-serialized crypto blobs):
//!   WrappedKey   MEK-wrapped vault key
//!   WrappedKey   Recovery-wrapped vault key
//!   SealedEnvelope  Encrypted metadata
//!
//! BODY:
//!   u32 LE       Item count
//!   For each item:
//!     SealedEnvelope  Encrypted item
//! ```

use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};

use super::envelope::{SealedEnvelope, decrypt_item, encrypt_item};
use super::errors::CryptoError;
use super::kdf::{Argon2Params, derive_encryption_key, derive_master_key};
use super::keys::{RecoveryKey, VaultKey};
use super::wrap::{
    WrappedKey, unwrap_vault_key, unwrap_vault_key_with_recovery, wrap_vault_key,
    wrap_vault_key_with_recovery,
};

// ── Constants ──────────────────────────────────────────────────────────────────

const MAGIC: &[u8; 4] = b"LDGR";
const FORMAT_VERSION: u16 = 1;
const KDF_VERSION: u8 = 1; // Argon2id v0x13
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const FIXED_HEADER_LEN: usize = 4 + 2 + 1 + SALT_LEN + 4 + 4 + 4; // 51 bytes

/// Maximum memory cost: 4 GiB (prevents denial-of-service from untrusted input).
const MAX_MEMORY_COST_KIB: u32 = 4 * 1024 * 1024;
/// Maximum iterations (time cost).
const MAX_ITERATIONS: u32 = 100;
/// Maximum parallelism threads.
const MAX_PARALLELISM: u32 = 16;
/// Maximum length for any single length-prefixed blob (1 MiB).
const MAX_BLOB_LEN: u32 = 1024 * 1024;
/// Maximum number of items in a vault file.
const MAX_ITEM_COUNT: u32 = 1_000_000;

// ── Types ──────────────────────────────────────────────────────────────────────

/// Metadata about the vault, encrypted in the header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultMetadata {
    /// Human-readable vault name (e.g. "Personal Finance").
    pub name: String,
    /// When the vault was created.
    pub created_at: DateTime<Utc>,
}

/// Parsed vault header. Contains all parameters needed to unlock the vault.
///
/// This struct can be obtained without the password (by parsing the file),
/// but the wrapped keys cannot be decrypted without the correct password
/// or recovery key.
#[derive(Debug, Clone)]
pub struct VaultHeader {
    pub format_version: u16,
    pub kdf_version: u8,
    pub salt: [u8; SALT_LEN],
    pub argon2_params: Argon2Params,
    pub wrapped_vk: WrappedKey,
    pub recovery_wrapped_vk: WrappedKey,
    pub encrypted_metadata: SealedEnvelope,
}

/// An unlocked vault with the vault key and decrypted metadata in memory.
///
/// Individual items remain encrypted as [`SealedEnvelope`]s — they are
/// decrypted on access via [`get_item`](UnlockedVault::get_item).
///
/// When this value is dropped, the [`VaultKey`] is zeroized automatically.
pub struct UnlockedVault {
    header: VaultHeader,
    vault_key: VaultKey,
    metadata: VaultMetadata,
    items: Vec<SealedEnvelope>,
}

impl std::fmt::Debug for UnlockedVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnlockedVault")
            .field("header", &self.header)
            .field("metadata", &self.metadata)
            .field("item_count", &self.items.len())
            .field("vault_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl UnlockedVault {
    /// Access the decrypted vault metadata.
    pub fn metadata(&self) -> &VaultMetadata {
        &self.metadata
    }

    /// Number of encrypted items in the vault.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Decrypt and return an item by index.
    ///
    /// # Errors
    ///
    /// Returns an error if decryption fails or the index is out of bounds.
    pub fn get_item(&self, index: usize) -> Result<Vec<u8>, CryptoError> {
        let envelope = self.items.get(index).ok_or_else(|| {
            CryptoError::InvalidVault(format!(
                "item index {index} out of bounds (count: {})",
                self.items.len()
            ))
        })?;
        decrypt_item(&self.vault_key, envelope)
    }

    /// Encrypt and add a new item to the vault.
    pub fn add_item(&mut self, plaintext: &[u8]) -> Result<(), CryptoError> {
        let envelope = encrypt_item(&self.vault_key, plaintext)?;
        self.items.push(envelope);
        Ok(())
    }

    /// Replace an existing item at the given index with new encrypted data.
    ///
    /// # Errors
    ///
    /// Returns an error if the index is out of bounds or encryption fails.
    pub fn replace_item(&mut self, index: usize, plaintext: &[u8]) -> Result<(), CryptoError> {
        if index >= self.items.len() {
            return Err(CryptoError::InvalidVault(format!(
                "item index {index} out of bounds (count: {})",
                self.items.len()
            )));
        }
        let envelope = encrypt_item(&self.vault_key, plaintext)?;
        self.items[index] = envelope;
        Ok(())
    }

    /// Remove all items from the vault.
    pub fn clear_items(&mut self) {
        self.items.clear();
    }

    /// Change the vault password, re-wrapping the vault key with a new MEK.
    ///
    /// Generates a new random salt and derives fresh keys from the new password.
    /// The vault key and recovery key wrapping remain the same underlying key —
    /// only the MEK wrapping is replaced.
    pub fn change_password(
        &mut self,
        new_password: &[u8],
        new_params: &Argon2Params,
    ) -> Result<(), CryptoError> {
        let mut new_salt = [0u8; SALT_LEN];
        rand::rng().fill_bytes(&mut new_salt);

        let mk = derive_master_key(new_password, &new_salt, new_params)?;
        let mek = derive_encryption_key(&mk)?;
        let new_wrapped_vk = wrap_vault_key(&mek, &self.vault_key)?;

        // Re-encrypt metadata with vault key (new envelope = new nonce)
        let metadata_json = serde_json::to_vec(&self.metadata)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;
        let new_encrypted_metadata = encrypt_item(&self.vault_key, &metadata_json)?;

        self.header.salt = new_salt;
        self.header.argon2_params = new_params.clone();
        self.header.wrapped_vk = new_wrapped_vk;
        self.header.encrypted_metadata = new_encrypted_metadata;

        Ok(())
    }

    /// Export the vault key bytes for session caching.
    ///
    /// **Security**: the caller must protect these bytes (restricted file
    /// permissions, OS keychain, secure enclave, etc.). The vault key gives
    /// full read/write access to all encrypted items.
    pub fn export_session_key(&self) -> [u8; 32] {
        *self.vault_key.as_bytes()
    }

    /// Access the vault format version.
    pub fn format_version(&self) -> u16 {
        self.header.format_version
    }
}

/// Restore an unlocked vault from a previously exported session key.
///
/// This skips password derivation — the caller provides the vault key
/// bytes directly (typically loaded from a session file or OS keychain).
///
/// # Errors
///
/// Returns an error if the vault format is invalid or the session key
/// cannot decrypt the metadata.
pub fn restore_vault_from_session(
    data: &[u8],
    session_key: &[u8; 32],
) -> Result<UnlockedVault, CryptoError> {
    let (header, items) = parse_vault(data)?;
    let vk = VaultKey::from_bytes(*session_key);

    let metadata_bytes = decrypt_item(&vk, &header.encrypted_metadata)?;
    let metadata: VaultMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|e| CryptoError::InvalidVault(format!("invalid metadata JSON: {e}")))?;

    Ok(UnlockedVault {
        header,
        vault_key: vk,
        metadata,
        items,
    })
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Create a new vault with the given password and name.
///
/// Generates a random vault key, a random recovery key, and wraps the vault
/// key with both the password-derived MEK and the recovery key. The metadata
/// is encrypted with the vault key.
///
/// Returns the unlocked vault and the recovery key. **The recovery key must
/// be displayed to the user immediately** — it cannot be retrieved later
/// without the vault being unlocked (see [`recovery_key`]).
///
/// # Errors
///
/// Returns an error if key derivation, wrapping, or encryption fails.
pub fn create_vault(
    password: &[u8],
    name: &str,
    params: &Argon2Params,
) -> Result<(UnlockedVault, RecoveryKey), CryptoError> {
    let mut salt = [0u8; SALT_LEN];
    rand::rng().fill_bytes(&mut salt);

    // Derive key hierarchy
    let mk = derive_master_key(password, &salt, params)?;
    let mek = derive_encryption_key(&mk)?;

    // Generate vault key and recovery key
    let vk = VaultKey::generate();
    let rk = RecoveryKey::generate();

    // Wrap vault key
    let wrapped_vk = wrap_vault_key(&mek, &vk)?;
    let recovery_wrapped_vk = wrap_vault_key_with_recovery(&rk, &vk)?;

    // Encrypt metadata
    let metadata = VaultMetadata {
        name: name.to_string(),
        created_at: Utc::now(),
    };
    let metadata_json =
        serde_json::to_vec(&metadata).map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;
    let encrypted_metadata = encrypt_item(&vk, &metadata_json)?;

    let header = VaultHeader {
        format_version: FORMAT_VERSION,
        kdf_version: KDF_VERSION,
        salt,
        argon2_params: params.clone(),
        wrapped_vk,
        recovery_wrapped_vk,
        encrypted_metadata,
    };

    let vault = UnlockedVault {
        header,
        vault_key: vk,
        metadata,
        items: Vec::new(),
    };

    Ok((vault, rk))
}

/// Open (unlock) a vault from its serialized bytes.
///
/// Parses the binary format, derives the key hierarchy from the password,
/// unwraps the vault key, and decrypts the metadata.
///
/// # Errors
///
/// Returns [`CryptoError::InvalidVault`] if the format is invalid,
/// [`CryptoError::UnsupportedVersion`] if the version is unknown,
/// or [`CryptoError::UnwrapFailed`] if the password is wrong.
pub fn open_vault(data: &[u8], password: &[u8]) -> Result<UnlockedVault, CryptoError> {
    let (header, items) = parse_vault(data)?;

    let mk = derive_master_key(password, &header.salt, &header.argon2_params)?;
    let mek = derive_encryption_key(&mk)?;
    let vk = unwrap_vault_key(&mek, &header.wrapped_vk)?;

    // Decrypt metadata
    let metadata_bytes = decrypt_item(&vk, &header.encrypted_metadata)?;
    let metadata: VaultMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|e| CryptoError::InvalidVault(format!("invalid metadata JSON: {e}")))?;

    Ok(UnlockedVault {
        header,
        vault_key: vk,
        metadata,
        items,
    })
}

/// Serialize an unlocked vault to bytes for writing to disk.
///
/// The output includes the full binary header (with wrapped keys and
/// encrypted metadata) followed by all encrypted items.
///
/// # Errors
///
/// Returns an error if serialization of any component fails.
pub fn serialize_vault(vault: &UnlockedVault) -> Result<Vec<u8>, CryptoError> {
    let mut w = BinaryWriter::new();

    write_header(&mut w, &vault.header)?;

    // Body: items
    let item_count = u32::try_from(vault.items.len())
        .map_err(|_| CryptoError::InvalidVault("too many items".into()))?;
    w.write_u32_le(item_count);

    for item in &vault.items {
        write_sealed_envelope(&mut w, item)?;
    }

    Ok(w.finish())
}

/// Validate that bytes represent a structurally valid vault file.
///
/// Checks the magic bytes and format version. Does **not** require a
/// password or attempt decryption.
///
/// # Errors
///
/// Returns [`CryptoError::InvalidVault`] if the magic bytes are wrong,
/// or [`CryptoError::UnsupportedVersion`] if the version is unknown.
pub fn validate_vault(data: &[u8]) -> Result<(), CryptoError> {
    if data.len() < FIXED_HEADER_LEN {
        return Err(CryptoError::InvalidVault("file too short".into()));
    }

    if &data[0..4] != MAGIC {
        return Err(CryptoError::InvalidVault(
            "not a vault file (invalid magic bytes)".into(),
        ));
    }

    let version = u16::from_le_bytes([data[4], data[5]]);
    if version != FORMAT_VERSION {
        return Err(CryptoError::UnsupportedVersion(version));
    }

    Ok(())
}

/// Recover a vault using the recovery key, setting a new password.
///
/// This function:
/// 1. Parses the vault header from bytes
/// 2. Unwraps the vault key using the recovery key
/// 3. Generates a new salt and derives fresh keys from the new password
/// 4. Re-wraps the vault key with the new MEK and re-wraps with the
///    same recovery key (fresh nonce)
/// 5. Re-encrypts metadata
///
/// # Errors
///
/// Returns [`CryptoError::UnwrapFailed`] if the recovery key is wrong,
/// or other crypto errors if key operations fail.
pub fn recover_vault(
    data: &[u8],
    recovery_key: &RecoveryKey,
    new_password: &[u8],
    new_params: &Argon2Params,
) -> Result<UnlockedVault, CryptoError> {
    let (header, items) = parse_vault(data)?;

    // Unwrap vault key with recovery key
    let vk = unwrap_vault_key_with_recovery(recovery_key, &header.recovery_wrapped_vk)?;

    // Decrypt metadata
    let metadata_bytes = decrypt_item(&vk, &header.encrypted_metadata)?;
    let metadata: VaultMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|e| CryptoError::InvalidVault(format!("invalid metadata JSON: {e}")))?;

    // Generate new salt and derive new key hierarchy
    let mut new_salt = [0u8; SALT_LEN];
    rand::rng().fill_bytes(&mut new_salt);

    let mk = derive_master_key(new_password, &new_salt, new_params)?;
    let mek = derive_encryption_key(&mk)?;

    // Re-wrap vault key with new MEK and same recovery key (fresh nonce)
    let new_wrapped_vk = wrap_vault_key(&mek, &vk)?;
    let new_recovery_wrapped_vk = wrap_vault_key_with_recovery(recovery_key, &vk)?;

    // Re-encrypt metadata
    let metadata_json =
        serde_json::to_vec(&metadata).map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;
    let new_encrypted_metadata = encrypt_item(&vk, &metadata_json)?;

    let new_header = VaultHeader {
        format_version: FORMAT_VERSION,
        kdf_version: KDF_VERSION,
        salt: new_salt,
        argon2_params: new_params.clone(),
        wrapped_vk: new_wrapped_vk,
        recovery_wrapped_vk: new_recovery_wrapped_vk,
        encrypted_metadata: new_encrypted_metadata,
    };

    Ok(UnlockedVault {
        header: new_header,
        vault_key: vk,
        metadata,
        items,
    })
}

/// Re-display the recovery key for an already-unlocked vault.
///
/// Extracts the vault key from the unlocked vault and wraps it with the
/// provided recovery key to verify it matches the stored recovery wrap.
/// If verification passes, returns `Ok(())`.
///
/// **Typical usage**: the CLI `ldgr recovery-kit` command calls this to
/// verify a recovery key entered by the user against the vault.
///
/// # Errors
///
/// Returns [`CryptoError::UnwrapFailed`] if the recovery key doesn't
/// match the recovery-wrapped vault key in the header.
pub fn verify_recovery_key(
    vault: &UnlockedVault,
    recovery_key: &RecoveryKey,
) -> Result<(), CryptoError> {
    let recovered_vk =
        unwrap_vault_key_with_recovery(recovery_key, &vault.header.recovery_wrapped_vk)?;
    if recovered_vk.as_bytes() != vault.vault_key.as_bytes() {
        return Err(CryptoError::UnwrapFailed);
    }
    Ok(())
}

// ── Deterministic test-vector serialization (feature = "test-vectors") ──────────

/// TEST ONLY — serialize an explicitly-constructed header and item list to the
/// v1 binary vault format.
///
/// Unlike [`serialize_vault`], this accepts a fully-formed [`VaultHeader`] and
/// item list directly, so callers can build a vault from known keys and nonces
/// and obtain byte-for-byte reproducible output for the published test vectors.
///
/// # Errors
///
/// Returns an error if any component is too large to encode.
#[cfg(feature = "test-vectors")]
#[doc(hidden)]
pub fn serialize_parts(
    header: &VaultHeader,
    items: &[SealedEnvelope],
) -> Result<Vec<u8>, CryptoError> {
    let mut w = BinaryWriter::new();

    write_header(&mut w, header)?;

    let item_count = u32::try_from(items.len())
        .map_err(|_| CryptoError::InvalidVault("too many items".into()))?;
    w.write_u32_le(item_count);

    for item in items {
        write_sealed_envelope(&mut w, item)?;
    }

    Ok(w.finish())
}

/// TEST ONLY — serialize a single [`WrappedKey`] to its v1 binary sub-format
/// (`version u8 || nonce[12] || ct_len u32 LE || ciphertext`).
#[cfg(feature = "test-vectors")]
#[doc(hidden)]
pub fn serialize_wrapped_key(wk: &WrappedKey) -> Result<Vec<u8>, CryptoError> {
    let mut w = BinaryWriter::new();
    write_wrapped_key(&mut w, wk)?;
    Ok(w.finish())
}

/// TEST ONLY — serialize a single [`SealedEnvelope`] to its v1 binary
/// sub-format (`version u8 || WrappedKey || nonce[12] || ct_len u32 LE || ciphertext`).
#[cfg(feature = "test-vectors")]
#[doc(hidden)]
pub fn serialize_sealed_envelope(env: &SealedEnvelope) -> Result<Vec<u8>, CryptoError> {
    let mut w = BinaryWriter::new();
    write_sealed_envelope(&mut w, env)?;
    Ok(w.finish())
}

// ── Internal: binary format parsing ────────────────────────────────────────────
/// Parse a complete vault from bytes into header + items.
fn parse_vault(data: &[u8]) -> Result<(VaultHeader, Vec<SealedEnvelope>), CryptoError> {
    validate_vault(data)?;

    let mut r = BinaryReader::new(data);

    // Fixed header
    r.skip(4)?; // magic (already validated)
    let format_version = r.read_u16_le()?;
    let kdf_version = r.read_u8()?;

    if kdf_version != KDF_VERSION {
        return Err(CryptoError::InvalidVault(format!(
            "unsupported KDF version: {kdf_version}"
        )));
    }

    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(r.read_bytes(SALT_LEN)?);

    let memory_cost_kib = r.read_u32_le()?;
    let iterations = r.read_u32_le()?;
    let parallelism = r.read_u32_le()?;

    // Enforce upper bounds on KDF params (untrusted input)
    validate_param_bounds(memory_cost_kib, iterations, parallelism)?;

    let argon2_params = Argon2Params {
        memory_cost_kib,
        iterations,
        parallelism,
    };

    // Variable header
    let wrapped_vk = read_wrapped_key(&mut r)?;
    let recovery_wrapped_vk = read_wrapped_key(&mut r)?;
    let encrypted_metadata = read_sealed_envelope(&mut r)?;

    let header = VaultHeader {
        format_version,
        kdf_version,
        salt,
        argon2_params,
        wrapped_vk,
        recovery_wrapped_vk,
        encrypted_metadata,
    };

    // Body
    let item_count = r.read_u32_le()?;
    if item_count > MAX_ITEM_COUNT {
        return Err(CryptoError::InvalidVault(format!(
            "item count {item_count} exceeds maximum {MAX_ITEM_COUNT}"
        )));
    }

    let mut items = Vec::with_capacity(item_count as usize);
    for _ in 0..item_count {
        items.push(read_sealed_envelope(&mut r)?);
    }

    Ok((header, items))
}

/// Enforce upper bounds on Argon2 parameters from untrusted input.
fn validate_param_bounds(
    memory_cost_kib: u32,
    iterations: u32,
    parallelism: u32,
) -> Result<(), CryptoError> {
    if memory_cost_kib > MAX_MEMORY_COST_KIB {
        return Err(CryptoError::InvalidVault(format!(
            "memory cost {memory_cost_kib} KiB exceeds maximum {MAX_MEMORY_COST_KIB} KiB"
        )));
    }
    if iterations > MAX_ITERATIONS {
        return Err(CryptoError::InvalidVault(format!(
            "iterations {iterations} exceeds maximum {MAX_ITERATIONS}"
        )));
    }
    if parallelism > MAX_PARALLELISM {
        return Err(CryptoError::InvalidVault(format!(
            "parallelism {parallelism} exceeds maximum {MAX_PARALLELISM}"
        )));
    }
    Ok(())
}

// ── Internal: binary serialization of crypto structs ───────────────────────────

fn write_header(w: &mut BinaryWriter, header: &VaultHeader) -> Result<(), CryptoError> {
    w.write_bytes(MAGIC);
    w.write_u16_le(header.format_version);
    w.write_u8(header.kdf_version);
    w.write_bytes(&header.salt);
    w.write_u32_le(header.argon2_params.memory_cost_kib);
    w.write_u32_le(header.argon2_params.iterations);
    w.write_u32_le(header.argon2_params.parallelism);

    write_wrapped_key(w, &header.wrapped_vk)?;
    write_wrapped_key(w, &header.recovery_wrapped_vk)?;
    write_sealed_envelope(w, &header.encrypted_metadata)?;

    Ok(())
}

/// Binary format for `WrappedKey`:
///   `version`: u8, `nonce`: `[u8; 12]`, `ct_len`: u32 LE, `ciphertext`: `[u8; ct_len]`
fn write_wrapped_key(w: &mut BinaryWriter, wk: &WrappedKey) -> Result<(), CryptoError> {
    w.write_u8(wk.version);
    w.write_bytes(&wk.nonce);
    let ct_len = u32::try_from(wk.ciphertext.len())
        .map_err(|_| CryptoError::InvalidVault("wrapped key ciphertext too large".into()))?;
    w.write_u32_le(ct_len);
    w.write_bytes(&wk.ciphertext);
    Ok(())
}

fn read_wrapped_key(r: &mut BinaryReader<'_>) -> Result<WrappedKey, CryptoError> {
    let version = r.read_u8()?;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(r.read_bytes(NONCE_LEN)?);
    let ct_len = r.read_u32_le()?;
    if ct_len > MAX_BLOB_LEN {
        return Err(CryptoError::InvalidVault(format!(
            "wrapped key ciphertext length {ct_len} exceeds maximum"
        )));
    }
    let ciphertext = r.read_bytes(ct_len as usize)?.to_vec();
    Ok(WrappedKey {
        version,
        nonce,
        ciphertext,
    })
}

/// Binary format for `SealedEnvelope`:
///   `version`: u8, `wrapped_ik`: `WrappedKey` (inline), `nonce`: `[u8; 12]`,
///   `ct_len`: u32 LE, `ciphertext`: `[u8; ct_len]`
fn write_sealed_envelope(w: &mut BinaryWriter, env: &SealedEnvelope) -> Result<(), CryptoError> {
    w.write_u8(env.version);
    write_wrapped_key(w, &env.wrapped_ik)?;
    w.write_bytes(&env.nonce);
    let ct_len = u32::try_from(env.ciphertext.len())
        .map_err(|_| CryptoError::InvalidVault("envelope ciphertext too large".into()))?;
    w.write_u32_le(ct_len);
    w.write_bytes(&env.ciphertext);
    Ok(())
}

fn read_sealed_envelope(r: &mut BinaryReader<'_>) -> Result<SealedEnvelope, CryptoError> {
    let version = r.read_u8()?;
    let wrapped_ik = read_wrapped_key(r)?;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(r.read_bytes(NONCE_LEN)?);
    let ct_len = r.read_u32_le()?;
    if ct_len > MAX_BLOB_LEN {
        return Err(CryptoError::InvalidVault(format!(
            "envelope ciphertext length {ct_len} exceeds maximum"
        )));
    }
    let ciphertext = r.read_bytes(ct_len as usize)?.to_vec();
    Ok(SealedEnvelope {
        version,
        wrapped_ik,
        nonce,
        ciphertext,
    })
}

// ── Binary I/O helpers ─────────────────────────────────────────────────────────

struct BinaryWriter {
    buf: Vec<u8>,
}

impl BinaryWriter {
    fn new() -> Self {
        Self {
            buf: Vec::with_capacity(4096),
        }
    }

    fn write_bytes(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    fn write_u16_le(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_u32_le(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn finish(self) -> Vec<u8> {
        self.buf
    }
}

struct BinaryReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BinaryReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], CryptoError> {
        if self.pos + n > self.data.len() {
            return Err(CryptoError::InvalidVault(format!(
                "unexpected end of data at offset {} (need {n} bytes, have {})",
                self.pos,
                self.data.len() - self.pos
            )));
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, CryptoError> {
        let bytes = self.read_bytes(1)?;
        Ok(bytes[0])
    }

    fn read_u16_le(&mut self) -> Result<u16, CryptoError> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32_le(&mut self) -> Result<u32, CryptoError> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn skip(&mut self, n: usize) -> Result<(), CryptoError> {
        self.read_bytes(n)?;
        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::recovery::{decode_recovery_key, encode_recovery_key};

    fn test_params() -> Argon2Params {
        Argon2Params::test()
    }

    // --- Create + serialize + open round-trip ---

    #[test]
    fn create_and_open_round_trip() {
        let password = b"test-password-123";
        let (vault, _rk) = create_vault(password, "Test Vault", &test_params()).unwrap();

        let bytes = serialize_vault(&vault).unwrap();
        let opened = open_vault(&bytes, password).unwrap();

        assert_eq!(opened.metadata().name, "Test Vault");
        assert_eq!(opened.item_count(), 0);
    }

    #[test]
    fn round_trip_with_items() {
        let password = b"items-test";
        let (mut vault, _rk) = create_vault(password, "Items Vault", &test_params()).unwrap();

        vault.add_item(b"transaction 1: groceries $42.50").unwrap();
        vault.add_item(b"transaction 2: rent $1500.00").unwrap();
        vault.add_item(b"").unwrap(); // empty item

        let bytes = serialize_vault(&vault).unwrap();
        let opened = open_vault(&bytes, password).unwrap();

        assert_eq!(opened.item_count(), 3);
        assert_eq!(
            opened.get_item(0).unwrap(),
            b"transaction 1: groceries $42.50"
        );
        assert_eq!(opened.get_item(1).unwrap(), b"transaction 2: rent $1500.00");
        assert_eq!(opened.get_item(2).unwrap(), b"");
    }

    // --- Password validation ---

    #[test]
    fn wrong_password_fails_gracefully() {
        let (vault, _rk) = create_vault(b"correct", "Vault", &test_params()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();

        let result = open_vault(&bytes, b"wrong");
        assert!(result.is_err());
        // Should be UnwrapFailed, not a crash
        match result.unwrap_err() {
            CryptoError::UnwrapFailed => {}
            other => panic!("expected UnwrapFailed, got: {other}"),
        }
    }

    // --- Vault validation ---

    #[test]
    fn validate_valid_vault() {
        let (vault, _rk) = create_vault(b"pass", "V", &test_params()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();
        assert!(validate_vault(&bytes).is_ok());
    }

    #[test]
    fn validate_rejects_non_vault() {
        assert!(validate_vault(b"NOT_A_VAULT").is_err());
    }

    #[test]
    fn validate_rejects_too_short() {
        assert!(validate_vault(b"LDG").is_err());
    }

    #[test]
    fn validate_rejects_wrong_version() {
        let (vault, _rk) = create_vault(b"p", "V", &test_params()).unwrap();
        let mut bytes = serialize_vault(&vault).unwrap();
        // Corrupt version to 99
        bytes[4] = 99;
        bytes[5] = 0;
        let result = validate_vault(&bytes);
        assert!(matches!(result, Err(CryptoError::UnsupportedVersion(99))));
    }

    // --- Corruption detection ---

    #[test]
    fn corrupted_header_detected() {
        let (vault, _rk) = create_vault(b"pass", "V", &test_params()).unwrap();
        let mut bytes = serialize_vault(&vault).unwrap();

        // Corrupt the salt
        bytes[10] ^= 0xFF;
        // Should fail to unwrap (wrong salt → wrong MK → wrong MEK)
        assert!(open_vault(&bytes, b"pass").is_err());
    }

    #[test]
    fn corrupted_wrapped_key_detected() {
        let (vault, _rk) = create_vault(b"pass", "V", &test_params()).unwrap();
        let mut bytes = serialize_vault(&vault).unwrap();

        // Corrupt a byte deep in the wrapped key area
        bytes[FIXED_HEADER_LEN + 5] ^= 0xFF;
        assert!(open_vault(&bytes, b"pass").is_err());
    }

    #[test]
    fn truncated_file_detected() {
        let (vault, _rk) = create_vault(b"pass", "V", &test_params()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();

        // Truncate at various points
        for len in [0, 4, 20, 50, FIXED_HEADER_LEN, bytes.len() / 2] {
            let truncated = &bytes[..len.min(bytes.len())];
            assert!(
                open_vault(truncated, b"pass").is_err(),
                "truncated to {len} bytes should fail"
            );
        }
    }

    // --- Metadata encryption ---

    #[test]
    fn metadata_is_encrypted_in_serialized_form() {
        let name = "My Secret Vault";
        let (vault, _rk) = create_vault(b"pass", name, &test_params()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();

        // The vault name should NOT appear in plaintext in the serialized bytes
        let bytes_str = String::from_utf8_lossy(&bytes);
        assert!(
            !bytes_str.contains(name),
            "vault name must not appear in plaintext"
        );
    }

    // --- Recovery flow ---

    #[test]
    fn recovery_key_unlocks_vault() {
        let password = b"original-password";
        let (vault, rk) = create_vault(password, "Recovery Test", &test_params()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();

        let recovered = recover_vault(&bytes, &rk, b"new-password", &test_params()).unwrap();
        assert_eq!(recovered.metadata().name, "Recovery Test");
    }

    #[test]
    fn recovery_then_open_with_new_password() {
        let (mut vault, rk) = create_vault(b"old", "Vault", &test_params()).unwrap();
        vault.add_item(b"important data").unwrap();
        let bytes = serialize_vault(&vault).unwrap();

        let recovered = recover_vault(&bytes, &rk, b"new-password", &test_params()).unwrap();
        let recovered_bytes = serialize_vault(&recovered).unwrap();

        // Old password no longer works
        assert!(open_vault(&recovered_bytes, b"old").is_err());

        // New password works and items are preserved
        let opened = open_vault(&recovered_bytes, b"new-password").unwrap();
        assert_eq!(opened.item_count(), 1);
        assert_eq!(opened.get_item(0).unwrap(), b"important data");
    }

    #[test]
    fn wrong_recovery_key_fails() {
        let (vault, _rk) = create_vault(b"pass", "V", &test_params()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();

        let wrong_rk = RecoveryKey::generate();
        let result = recover_vault(&bytes, &wrong_rk, b"new", &test_params());
        assert!(result.is_err());
    }

    #[test]
    fn lost_password_and_recovery_key_is_unrecoverable() {
        let (vault, _rk) = create_vault(b"forgotten", "V", &test_params()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();

        // Wrong password
        assert!(open_vault(&bytes, b"wrong").is_err());
        // Wrong recovery key
        let wrong_rk = RecoveryKey::generate();
        assert!(recover_vault(&bytes, &wrong_rk, b"new", &test_params()).is_err());
    }

    // --- Recovery key display ---

    #[test]
    fn recovery_key_round_trip_via_display() {
        let (vault, rk) = create_vault(b"pass", "V", &test_params()).unwrap();

        // Encode recovery key as human-readable string
        let encoded = encode_recovery_key(&rk);
        // Decode it back
        let decoded = decode_recovery_key(&encoded).unwrap();
        // Verify it can unlock the vault
        assert!(verify_recovery_key(&vault, &decoded).is_ok());
    }

    // --- Verify recovery key ---

    #[test]
    fn verify_recovery_key_correct() {
        let (vault, rk) = create_vault(b"pass", "V", &test_params()).unwrap();
        assert!(verify_recovery_key(&vault, &rk).is_ok());
    }

    #[test]
    fn verify_recovery_key_wrong() {
        let (vault, _rk) = create_vault(b"pass", "V", &test_params()).unwrap();
        let wrong_rk = RecoveryKey::generate();
        assert!(verify_recovery_key(&vault, &wrong_rk).is_err());
    }

    // --- Password change ---

    #[test]
    fn change_password_works() {
        let (mut vault, _rk) = create_vault(b"old-pass", "V", &test_params()).unwrap();
        vault.add_item(b"data").unwrap();

        vault.change_password(b"new-pass", &test_params()).unwrap();

        let bytes = serialize_vault(&vault).unwrap();
        // Old password fails
        assert!(open_vault(&bytes, b"old-pass").is_err());
        // New password works
        let opened = open_vault(&bytes, b"new-pass").unwrap();
        assert_eq!(opened.get_item(0).unwrap(), b"data");
    }

    // --- Magic bytes ---

    #[test]
    fn serialized_vault_starts_with_magic() {
        let (vault, _rk) = create_vault(b"p", "V", &test_params()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();
        assert_eq!(&bytes[0..4], b"LDGR");
    }

    #[test]
    fn serialized_vault_has_correct_version() {
        let (vault, _rk) = create_vault(b"p", "V", &test_params()).unwrap();
        let bytes = serialize_vault(&vault).unwrap();
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        assert_eq!(version, FORMAT_VERSION);
    }

    // --- Resource exhaustion protection ---

    #[test]
    fn rejects_excessive_argon2_memory() {
        let (vault, _rk) = create_vault(b"p", "V", &test_params()).unwrap();
        let mut bytes = serialize_vault(&vault).unwrap();

        // Set memory cost to u32::MAX
        let huge_mem = u32::MAX.to_le_bytes();
        bytes[39..43].copy_from_slice(&huge_mem);
        assert!(open_vault(&bytes, b"p").is_err());
    }

    #[test]
    fn rejects_excessive_iterations() {
        let (vault, _rk) = create_vault(b"p", "V", &test_params()).unwrap();
        let mut bytes = serialize_vault(&vault).unwrap();

        let huge_iter = (MAX_ITERATIONS + 1).to_le_bytes();
        bytes[43..47].copy_from_slice(&huge_iter);
        assert!(open_vault(&bytes, b"p").is_err());
    }

    // --- Property-based tests ---

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        fn arb_password() -> impl Strategy<Value = Vec<u8>> {
            proptest::collection::vec(any::<u8>(), 1..64)
        }

        fn arb_vault_name() -> impl Strategy<Value = String> {
            "[a-zA-Z0-9 _-]{1,32}"
        }

        proptest! {
            #[test]
            fn vault_create_save_open_round_trip(
                password in arb_password(),
                name in arb_vault_name(),
            ) {
                let (vault, _rk) = create_vault(&password, &name, &test_params()).unwrap();
                let bytes = serialize_vault(&vault).unwrap();
                let opened = open_vault(&bytes, &password).unwrap();

                prop_assert_eq!(opened.metadata().name.as_str(), name.as_str());
                prop_assert_eq!(opened.item_count(), 0);
            }

            #[test]
            fn vault_round_trip_preserves_items(
                password in arb_password(),
                items in proptest::collection::vec(
                    proptest::collection::vec(any::<u8>(), 0..512),
                    0..5,
                ),
            ) {
                let (mut vault, _rk) = create_vault(&password, "test", &test_params()).unwrap();
                for item in &items {
                    vault.add_item(item).unwrap();
                }

                let bytes = serialize_vault(&vault).unwrap();
                let opened = open_vault(&bytes, &password).unwrap();

                prop_assert_eq!(opened.item_count(), items.len());
                for (i, expected) in items.iter().enumerate() {
                    prop_assert_eq!(&opened.get_item(i).unwrap(), expected);
                }
            }

            #[test]
            fn recovery_flow_preserves_vault_contents(
                old_password in arb_password(),
                new_password in arb_password(),
                items in proptest::collection::vec(
                    proptest::collection::vec(any::<u8>(), 1..256),
                    1..4,
                ),
            ) {
                let (mut vault, rk) = create_vault(&old_password, "recovery", &test_params()).unwrap();
                for item in &items {
                    vault.add_item(item).unwrap();
                }

                let bytes = serialize_vault(&vault).unwrap();
                let recovered = recover_vault(&bytes, &rk, &new_password, &test_params()).unwrap();
                let recovered_bytes = serialize_vault(&recovered).unwrap();

                // Old password must fail
                prop_assert!(open_vault(&recovered_bytes, &old_password).is_err());

                // New password must work and preserve all items
                let opened = open_vault(&recovered_bytes, &new_password).unwrap();
                prop_assert_eq!(opened.metadata().name.as_str(), "recovery");
                prop_assert_eq!(opened.item_count(), items.len());
                for (i, expected) in items.iter().enumerate() {
                    prop_assert_eq!(&opened.get_item(i).unwrap(), expected);
                }
            }

            #[test]
            fn wrong_password_always_fails(
                correct in arb_password(),
                wrong in arb_password(),
            ) {
                prop_assume!(correct != wrong);
                let (vault, _rk) = create_vault(&correct, "V", &test_params()).unwrap();
                let bytes = serialize_vault(&vault).unwrap();
                prop_assert!(open_vault(&bytes, &wrong).is_err());
            }

            #[test]
            fn session_key_round_trip(password in arb_password()) {
                let (vault, _rk) = create_vault(&password, "session", &test_params()).unwrap();
                let session_key = vault.export_session_key();
                let bytes = serialize_vault(&vault).unwrap();

                let restored = restore_vault_from_session(&bytes, &session_key).unwrap();
                prop_assert_eq!(restored.metadata().name.as_str(), "session");
            }
        }
    }
}
