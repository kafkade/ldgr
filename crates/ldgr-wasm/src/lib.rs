//! WASM bindings for ldgr-core.
//!
//! Exposes vault crypto operations, journal parsing, and balance computation
//! to JavaScript via `wasm-bindgen`. Storage is handled on the JS side
//! (e.g., sql.js + `IndexedDB`) — this module provides pure computation only.

// wasm-bindgen uses #[unsafe(no_mangle)] for exports.
#![allow(unsafe_code)]
// wasm-bindgen requires owned values at the JS boundary.
#![allow(clippy::needless_pass_by_value)]

use wasm_bindgen::prelude::*;

use ldgr_core::accounting::{parser, reports, types as acct};
use ldgr_core::crypto::{self, Argon2Params};

// ── Error Helpers ──────────────────────────────────────────────────────────────

fn crypto_err(e: crypto::CryptoError) -> JsError {
    JsError::new(&format!("crypto error: {e}"))
}

// ── Vault Operations ───────────────────────────────────────────────────────────

/// Result of creating a new vault — contains the serialized vault blob
/// and the recovery key the user must save.
#[wasm_bindgen]
pub struct CreateVaultResult {
    vault_data: Vec<u8>,
    recovery_key: String,
}

#[wasm_bindgen]
impl CreateVaultResult {
    /// Serialized vault blob — store this encrypted in `IndexedDB`/OPFS.
    #[wasm_bindgen(getter, js_name = vaultData)]
    pub fn vault_data(&self) -> Vec<u8> {
        self.vault_data.clone()
    }

    /// Recovery key — present to user immediately; cannot be retrieved later.
    #[wasm_bindgen(getter, js_name = recoveryKey)]
    pub fn recovery_key(&self) -> String {
        self.recovery_key.clone()
    }
}

/// An unlocked vault handle holding decrypted keys in memory.
///
/// JS owns persistence (`IndexedDB`/sql.js). This struct provides
/// crypto operations and vault item management.
#[wasm_bindgen]
pub struct LdgrWasm {
    vault: crypto::UnlockedVault,
}

#[wasm_bindgen]
impl LdgrWasm {
    /// Create a new vault with the given password and name.
    ///
    /// Returns a `CreateVaultResult` containing the vault blob and recovery key.
    /// Uses WASM-optimized Argon2id parameters (64 MB, 3 iterations).
    #[wasm_bindgen(js_name = createVault)]
    pub fn create_vault(password: &str, name: &str) -> Result<CreateVaultResult, JsError> {
        let (vault, recovery_key) =
            crypto::create_vault(password.as_bytes(), name, &Argon2Params::wasm())
                .map_err(crypto_err)?;

        let vault_data = crypto::serialize_vault(&vault).map_err(crypto_err)?;
        let recovery_string = crypto::encode_recovery_key(&recovery_key);

        Ok(CreateVaultResult {
            vault_data,
            recovery_key: recovery_string,
        })
    }

    /// Open an existing vault with the given password.
    ///
    /// `vault_data` is the serialized vault blob (from `CreateVaultResult.vaultData`
    /// or loaded from storage).
    #[wasm_bindgen(js_name = openVault)]
    pub fn open_vault(vault_data: &[u8], password: &str) -> Result<LdgrWasm, JsError> {
        let vault = crypto::open_vault(vault_data, password.as_bytes()).map_err(crypto_err)?;
        Ok(Self { vault })
    }

    /// Get the vault name.
    #[wasm_bindgen(js_name = vaultName)]
    pub fn vault_name(&self) -> String {
        self.vault.metadata().name.clone()
    }

    /// Encrypt and add an item to the vault. Each item gets a unique per-item key.
    #[wasm_bindgen(js_name = addItem)]
    pub fn add_item(&mut self, plaintext: &[u8]) -> Result<(), JsError> {
        self.vault.add_item(plaintext).map_err(crypto_err)
    }

    /// Decrypt and return an item by index.
    #[wasm_bindgen(js_name = getItem)]
    pub fn get_item(&self, index: usize) -> Result<Vec<u8>, JsError> {
        self.vault.get_item(index).map_err(crypto_err)
    }

    /// Replace an existing item at the given index with new encrypted data.
    #[wasm_bindgen(js_name = replaceItem)]
    pub fn replace_item(&mut self, index: usize, plaintext: &[u8]) -> Result<(), JsError> {
        self.vault
            .replace_item(index, plaintext)
            .map_err(crypto_err)
    }

    /// Remove all items from the vault.
    #[wasm_bindgen(js_name = clearItems)]
    pub fn clear_items(&mut self) {
        self.vault.clear_items();
    }

    /// Number of encrypted items in the vault.
    #[wasm_bindgen(js_name = itemCount)]
    pub fn item_count(&self) -> usize {
        self.vault.item_count()
    }

    /// Re-serialize the vault (e.g., after adding items). Returns the vault blob.
    #[wasm_bindgen(js_name = serializeVault)]
    pub fn serialize_vault(&self) -> Result<Vec<u8>, JsError> {
        crypto::serialize_vault(&self.vault).map_err(crypto_err)
    }
}

// ── Journal Parsing ────────────────────────────────────────────────────────────

/// Parse an hledger journal string into transactions.
///
/// Returns a JSON string of the parsed transactions on success,
/// or throws with parse error details.
#[wasm_bindgen(js_name = parseJournal)]
pub fn parse_journal(text: &str) -> Result<String, JsError> {
    let journal = parser::parse_journal(text).map_err(|errors| {
        let msgs: Vec<String> = errors
            .iter()
            .map(|e| format!("line {}: {}", e.line, e.message))
            .collect();
        JsError::new(&msgs.join("\n"))
    })?;

    serde_json::to_string(&journal.transactions)
        .map_err(|e| JsError::new(&format!("serialization error: {e}")))
}

// ── Balance Report ─────────────────────────────────────────────────────────────

/// Compute a balance report from a JSON array of transactions.
///
/// `transactions_json` must be a JSON string of the transactions array
/// (as returned by `parseJournal`). Optional filters narrow the results.
/// Returns a JSON string of the balance report.
#[wasm_bindgen(js_name = computeBalance)]
pub fn compute_balance(
    transactions_json: &str,
    account_filter: Option<String>,
    begin_date: Option<String>,
    end_date: Option<String>,
) -> Result<String, JsError> {
    let txns: Vec<acct::Transaction> = serde_json::from_str(transactions_json)
        .map_err(|e| JsError::new(&format!("invalid transactions JSON: {e}")))?;

    let report = reports::compute_balance(
        &txns,
        account_filter.as_deref(),
        begin_date.as_deref(),
        end_date.as_deref(),
    );

    serde_json::to_string(&report).map_err(|e| JsError::new(&format!("serialization error: {e}")))
}

/// Compute a register report from a JSON array of transactions.
///
/// Returns a JSON string of the register report.
#[wasm_bindgen(js_name = computeRegister)]
pub fn compute_register(
    transactions_json: &str,
    account_filter: Option<String>,
    begin_date: Option<String>,
    end_date: Option<String>,
) -> Result<String, JsError> {
    let txns: Vec<acct::Transaction> = serde_json::from_str(transactions_json)
        .map_err(|e| JsError::new(&format!("invalid transactions JSON: {e}")))?;

    let report = reports::compute_register(
        &txns,
        account_filter.as_deref(),
        begin_date.as_deref(),
        end_date.as_deref(),
    );

    serde_json::to_string(&report).map_err(|e| JsError::new(&format!("serialization error: {e}")))
}

// ── WASM Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn vault_round_trip() {
        let result = LdgrWasm::create_vault("test-password", "Test Vault").unwrap();
        assert!(!result.recovery_key().is_empty());
        assert!(!result.vault_data().is_empty());

        let vault = LdgrWasm::open_vault(&result.vault_data(), "test-password").unwrap();
        assert_eq!(vault.vault_name(), "Test Vault");
    }

    #[wasm_bindgen_test]
    fn item_encrypt_decrypt_round_trip() {
        let result = LdgrWasm::create_vault("pw", "V").unwrap();
        let mut vault = LdgrWasm::open_vault(&result.vault_data(), "pw").unwrap();

        vault.add_item(b"hello, ledger!").unwrap();
        assert_eq!(vault.item_count(), 1);

        let decrypted = vault.get_item(0).unwrap();
        assert_eq!(&decrypted, b"hello, ledger!");
    }

    #[wasm_bindgen_test]
    fn parse_journal_basic() {
        let journal = r"
2024-01-15 * Grocery store
    Expenses:Food    $50.00
    Assets:Checking
";
        let result = parse_journal(journal);
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.contains("Grocery store"));
    }

    #[wasm_bindgen_test]
    fn wrong_password_fails() {
        let result = LdgrWasm::create_vault("correct", "V").unwrap();
        let err = LdgrWasm::open_vault(&result.vault_data(), "wrong");
        assert!(err.is_err());
    }

    #[wasm_bindgen_test]
    fn vault_serialize_round_trip() {
        let result = LdgrWasm::create_vault("pw", "V").unwrap();
        let mut vault = LdgrWasm::open_vault(&result.vault_data(), "pw").unwrap();
        vault.add_item(b"test data").unwrap();

        let serialized = vault.serialize_vault().unwrap();
        let vault2 = LdgrWasm::open_vault(&serialized, "pw").unwrap();
        assert_eq!(vault2.item_count(), 1);
        assert_eq!(vault2.get_item(0).unwrap(), b"test data");
    }
}
