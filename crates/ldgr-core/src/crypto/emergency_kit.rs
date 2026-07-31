//! Account **Emergency Kit** — a printable/QR artifact for new-device server
//! sign-in (ADR-008, Decision 4).
//!
//! The kit is generated client-side at account creation and is scoped to
//! **server account recovery**. It bundles the data a user needs to sign in on
//! a fresh device: the sign-in address (server URL), the account identity
//! (email/username), and the account [`SecretKey`]. Core produces only a
//! render-agnostic struct + serialization; platforms turn it into a printable
//! PDF/QR image.
//!
//! # The three secrets — do not confuse them
//!
//! | Secret | Trust domain | Purpose | In this kit? |
//! | --- | --- | --- | --- |
//! | **Master password** | vault + account | Opens the vault; one of two SRP inputs | **No** — memorized, never printed |
//! | **Vault recovery key** (52-char Crockford) | vault only | Unwraps the `VaultKey` *offline* if the password is forgotten | **No** by default (opt-in only) |
//! | **Account Secret Key** (`A1-…`) | account/sync only | Second SRP factor for *server* sign-in on a new device | **Yes** |
//!
//! Per ADR-008, the account Secret Key and the vault recovery key are **kept as
//! separate artifacts** with different blast radius: the Secret Key gates
//! *server access*; the vault recovery key gates *plaintext data*. The kit
//! therefore omits the vault recovery key unless the caller explicitly opts in
//! ([`EmergencyKit::with_recovery_key`]). The master password is never included.

use serde::{Deserialize, Serialize};

use super::account_kdf::AccountKdf;
use super::errors::CryptoError;
use super::keys::RecoveryKey;
use super::recovery::{decode_recovery_key, encode_recovery_key};
use super::secret_key::SecretKey;

/// Schema version for the kit + QR payload. Bump for breaking changes.
///
/// v2 (#296) adds the account-scoped Argon2 [`AccountKdf`] needed to derive
/// `MK_auth` on a new device without the original vault header.
const KIT_VERSION: u32 = 2;

/// An account Emergency Kit: render-agnostic onboarding data for new-device
/// server sign-in.
///
/// Holds text forms only (so it serializes cleanly and stays render-agnostic).
/// The account Secret Key is always present; the vault recovery key is opt-in
/// and absent by default. Secret fields are redacted in [`Debug`].
#[derive(Clone, Serialize, Deserialize)]
pub struct EmergencyKit {
    /// Schema version of this kit.
    version: u32,
    /// Sign-in address: the self-host server URL (e.g. `https://ledger.example.org`).
    address: String,
    /// Account identity: email or username (the SRP sign-in identifier).
    email: String,
    /// Non-secret account-id hint (mirrors `SecretKey::account_hint`).
    account_hint: String,
    /// Account Secret Key in canonical `A1-…` text form. Secret.
    secret_key: String,
    /// Account-scoped Argon2id salt + params for deriving `MK_auth` on a new
    /// device (ADR-008 / #296). Not secret — no more sensitive than the SRP
    /// salt the server returns pre-auth — but carried here so onboarding works
    /// offline and as a cross-check against a substituted-salt server.
    account_kdf: AccountKdf,
    /// Optional vault recovery key text. Absent by default (ADR-008). Secret.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    recovery_key: Option<String>,
}

impl EmergencyKit {
    /// Build a kit from account-creation outputs: the sign-in `address`, the
    /// account `email`/username, the account `secret_key`, and the account-scoped
    /// `account_kdf` (used to derive `MK_auth` on a new device). The vault
    /// recovery key is **not** included (use [`with_recovery_key`] to opt in).
    ///
    /// [`with_recovery_key`]: EmergencyKit::with_recovery_key
    #[must_use]
    pub fn new(
        address: impl Into<String>,
        email: impl Into<String>,
        secret_key: &SecretKey,
        account_kdf: &AccountKdf,
    ) -> Self {
        Self {
            version: KIT_VERSION,
            address: address.into(),
            email: email.into(),
            account_hint: secret_key.account_hint().to_string(),
            secret_key: secret_key.encode(),
            account_kdf: account_kdf.clone(),
            recovery_key: None,
        }
    }

    /// Opt in to bundling the **vault recovery key** alongside the account
    /// Secret Key.
    ///
    /// ADR-008 recommends keeping the two recovery artifacts separate (one
    /// stolen sheet otherwise compromises both factors). Use only when the
    /// caller intentionally wants a single combined sheet.
    #[must_use]
    pub fn with_recovery_key(mut self, recovery_key: &RecoveryKey) -> Self {
        self.recovery_key = Some(encode_recovery_key(recovery_key));
        self
    }

    /// Schema version of this kit.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Sign-in address (server URL).
    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Account identity (email/username).
    #[must_use]
    pub fn email(&self) -> &str {
        &self.email
    }

    /// Non-secret account-id hint.
    #[must_use]
    pub fn account_hint(&self) -> &str {
        &self.account_hint
    }

    /// Account Secret Key in canonical `A1-…` text form. Secret material.
    #[must_use]
    pub fn secret_key_text(&self) -> &str {
        &self.secret_key
    }

    /// The account-scoped Argon2id KDF (salt + params) for deriving `MK_auth`
    /// on a new device (#296). Non-secret.
    #[must_use]
    pub fn account_kdf(&self) -> &AccountKdf {
        &self.account_kdf
    }

    /// The vault recovery key text, if the kit bundles one. Secret material.
    #[must_use]
    pub fn recovery_key_text(&self) -> Option<&str> {
        self.recovery_key.as_deref()
    }

    /// Parse the account Secret Key back into a typed [`SecretKey`].
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidParams`] if the stored text is malformed.
    pub fn secret_key(&self) -> Result<SecretKey, CryptoError> {
        SecretKey::parse(&self.secret_key)
    }

    /// Parse the bundled vault recovery key, if present.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidParams`] if a stored key is malformed.
    pub fn recovery_key(&self) -> Result<Option<RecoveryKey>, CryptoError> {
        self.recovery_key
            .as_deref()
            .map(decode_recovery_key)
            .transpose()
    }

    /// Encode the kit as its QR-encodable, versioned JSON payload. Platforms
    /// render this string into the kit's QR code for fast new-device sign-in.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidParams`] if serialization fails.
    pub fn to_qr_payload(&self) -> Result<String, CryptoError> {
        serde_json::to_string(self)
            .map_err(|e| CryptoError::InvalidParams(format!("kit serialization failed: {e}")))
    }

    /// Parse a scanned/typed QR payload back into an [`EmergencyKit`].
    ///
    /// Validates the schema version and the embedded account Secret Key so a
    /// caller gets the values needed to add a new device.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::InvalidParams`] if the JSON is malformed, the
    /// version is unsupported, or the Secret Key fails to parse.
    pub fn from_qr_payload(payload: &str) -> Result<Self, CryptoError> {
        let kit: Self = serde_json::from_str(payload)
            .map_err(|e| CryptoError::InvalidParams(format!("invalid kit payload: {e}")))?;
        if kit.version != KIT_VERSION {
            return Err(CryptoError::InvalidParams(format!(
                "unsupported emergency kit version {} (expected {KIT_VERSION})",
                kit.version
            )));
        }
        // Validate the secret key round-trips so callers fail fast.
        kit.secret_key()?;
        kit.recovery_key()?;
        Ok(kit)
    }
}

impl std::fmt::Debug for EmergencyKit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmergencyKit")
            .field("version", &self.version)
            .field("address", &self.address)
            .field("email", &self.email)
            .field("account_hint", &self.account_hint)
            .field("secret_key", &"[REDACTED]")
            .field("account_kdf", &self.account_kdf)
            .field(
                "recovery_key",
                &self.recovery_key.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Argon2Params;
    use uuid::Uuid;

    fn fixed_uuid() -> Uuid {
        Uuid::from_bytes([9u8; 16])
    }

    fn test_kdf() -> AccountKdf {
        AccountKdf::from_parts(vec![3u8; 16], Argon2Params::test())
    }

    fn kit() -> EmergencyKit {
        let sk = SecretKey::generate(fixed_uuid());
        EmergencyKit::new(
            "https://ledger.example.org",
            "user@example.org",
            &sk,
            &test_kdf(),
        )
    }

    #[test]
    fn new_bundles_secret_key_not_recovery() {
        let k = kit();
        assert_eq!(k.address(), "https://ledger.example.org");
        assert_eq!(k.email(), "user@example.org");
        assert!(k.secret_key_text().starts_with("A1-"));
        assert_eq!(k.account_kdf(), &test_kdf());
        assert!(k.recovery_key_text().is_none());
        assert!(k.recovery_key().unwrap().is_none());
    }

    #[test]
    fn account_hint_matches_secret_key() {
        let sk = SecretKey::generate(fixed_uuid());
        let k = EmergencyKit::new("a", "b", &sk, &test_kdf());
        assert_eq!(k.account_hint(), sk.account_hint());
    }

    #[test]
    fn qr_round_trip_secret_key_only() {
        let k = kit();
        let payload = k.to_qr_payload().unwrap();
        let parsed = EmergencyKit::from_qr_payload(&payload).unwrap();
        assert_eq!(parsed.address(), k.address());
        assert_eq!(parsed.email(), k.email());
        assert_eq!(parsed.secret_key_text(), k.secret_key_text());
        assert_eq!(parsed.account_kdf(), k.account_kdf());
        assert_eq!(
            parsed.secret_key().unwrap().encode(),
            k.secret_key().unwrap().encode()
        );
        assert!(parsed.recovery_key_text().is_none());
    }

    #[test]
    fn qr_round_trip_with_recovery_key() {
        let rk = RecoveryKey::generate();
        let k = kit().with_recovery_key(&rk);
        let payload = k.to_qr_payload().unwrap();
        let parsed = EmergencyKit::from_qr_payload(&payload).unwrap();
        let parsed_rk = parsed.recovery_key().unwrap().unwrap();
        assert_eq!(parsed_rk.as_bytes(), rk.as_bytes());
    }

    #[test]
    fn payload_omits_recovery_key_when_absent() {
        let payload = kit().to_qr_payload().unwrap();
        assert!(!payload.contains("recovery_key"));
    }

    #[test]
    fn parse_helpers_return_typed_values() {
        let sk = SecretKey::generate(fixed_uuid());
        let k = EmergencyKit::new("a", "b", &sk, &test_kdf());
        assert_eq!(k.secret_key().unwrap().body(), sk.body());
    }

    #[test]
    fn from_payload_rejects_bad_version() {
        let payload = kit()
            .to_qr_payload()
            .unwrap()
            .replace("\"version\":2", "\"version\":99");
        assert!(EmergencyKit::from_qr_payload(&payload).is_err());
    }

    #[test]
    fn from_payload_rejects_malformed_secret_key() {
        let bad = kit().to_qr_payload().unwrap().replace("A1-", "B1-");
        assert!(EmergencyKit::from_qr_payload(&bad).is_err());
    }

    #[test]
    fn from_payload_rejects_garbage() {
        assert!(EmergencyKit::from_qr_payload("not json").is_err());
    }

    #[test]
    fn debug_redacts_secrets() {
        let rk = RecoveryKey::generate();
        let k = kit().with_recovery_key(&rk);
        let dbg = format!("{k:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains(k.secret_key_text()));
        assert!(!dbg.contains(&encode_recovery_key(&rk)));
        // Non-secret identity fields are visible.
        assert!(dbg.contains("user@example.org"));
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn round_trip_arbitrary_account(
                bytes in proptest::array::uniform16(any::<u8>()),
                with_rk in any::<bool>(),
            ) {
                let sk = SecretKey::generate(Uuid::from_bytes(bytes));
                let mut k = EmergencyKit::new("https://h", "id", &sk, &test_kdf());
                if with_rk {
                    k = k.with_recovery_key(&RecoveryKey::generate());
                }
                let parsed = EmergencyKit::from_qr_payload(&k.to_qr_payload().unwrap()).unwrap();
                let parsed_sk = parsed.secret_key().unwrap();
                prop_assert_eq!(parsed_sk.body(), sk.body());
                prop_assert_eq!(parsed.recovery_key_text().is_some(), with_rk);
            }
        }
    }
}
