//! Relay-backed device pairing orchestration.
//!
//! Composes the X25519 onboarding crypto ([`super::onboarding`]) with the
//! server key-exchange relay ([`ServerSyncClient`]) into a **two-offer**
//! handshake that transfers the vault key from an existing device to a new one.
//!
//! A single relay offer only carries one offer + one (write-once) response, so a
//! three-message key exchange (pubA → pubB → encrypted key) is mapped onto two
//! offers. Both devices authenticate as the **same account** (ADR-003), so each
//! can create offers and read/write the other's:
//!
//! 1. The **initiator** creates `offer1` and ships its ephemeral public key +
//!    the `offer1` id out-of-band as a [`PairingCode`] (QR / copy-paste).
//! 2. The **joiner** derives the shared secret, creates `offer2` (its return
//!    mailbox), and posts `{pubB, offer2}` as the response to `offer1`.
//! 3. The initiator reads that, encrypts the vault key under the shared secret,
//!    and posts the ciphertext as the response to `offer2`.
//! 4. The joiner reads `offer2`'s response and decrypts the vault key.
//!
//! The relay only ever sees ciphertext — the vault key is never on the wire in
//! plaintext. This module is pure orchestration over an injected transport
//! (`RawHttpSender`); the CLI owns the poll loop, QR rendering, and all I/O.

use serde::{Deserialize, Serialize};
use x25519_dalek::EphemeralSecret;
use zeroize::Zeroize;

use super::onboarding::{
    QrPayload, base64_decode, base64_encode, complete_onboarding, decrypt_vault_key,
    encrypt_vault_key, initiate_onboarding, respond_to_onboarding,
};
use super::server::{RawHttpSender, ServerSyncClient, ServerSyncError};

/// Errors raised while orchestrating a device pairing.
#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    /// The underlying server transport or relay call failed.
    #[error(transparent)]
    Server(#[from] ServerSyncError),
    /// A pairing code or relay payload could not be parsed.
    #[error("invalid pairing payload: {0}")]
    InvalidPayload(String),
    /// The X25519 key exchange (shared-secret derivation or (de)cryption) failed.
    #[error("key exchange failed: {0}")]
    KeyExchange(String),
}

/// The out-of-band token shown by the initiating device and consumed by the
/// joining device (`ldgr devices join <code>`).
///
/// It carries the initiator's ephemeral X25519 public key, the relay `offer1`
/// id the joiner posts its hello to, connection info, and a short verification
/// code for MITM confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingCode {
    /// Relay offer id the joiner posts its hello to.
    pub offer_id: String,
    /// Initiator ephemeral X25519 public key (base64, 32 bytes).
    pub public_key: String,
    /// Connection info (relay server URL).
    pub connection: String,
    /// Six-digit verification code for MITM prevention.
    pub verification_code: String,
}

impl PairingCode {
    /// Encode to a compact single-token string (base64 of the JSON body) that is
    /// convenient to embed in a QR code or copy/paste.
    #[must_use]
    pub fn encode(&self) -> String {
        // serde_json cannot fail for this plain struct of strings.
        let json = serde_json::to_vec(self).unwrap_or_default();
        base64_encode(&json)
    }

    /// Decode a token produced by [`encode`](Self::encode).
    ///
    /// # Errors
    ///
    /// Returns [`PairingError::InvalidPayload`] if the token is not valid base64
    /// or does not deserialize into a [`PairingCode`].
    pub fn decode(token: &str) -> Result<Self, PairingError> {
        let json = base64_decode(token.trim()).map_err(PairingError::InvalidPayload)?;
        serde_json::from_slice(&json).map_err(|e| PairingError::InvalidPayload(e.to_string()))
    }
}

/// The joiner's hello, posted as the response to `offer1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JoinerHello {
    /// Joiner ephemeral X25519 public key (base64).
    public_key: String,
    /// The joiner's return-mailbox offer id (`offer2`) the initiator posts the
    /// encrypted vault key to.
    response_offer_id: String,
}

/// State the initiating device keeps between [`initiate_pairing`] and
/// [`deliver_vault_key`].
pub struct Initiation {
    /// Ephemeral secret kept in memory; consumed to derive the shared secret.
    secret: EphemeralSecret,
    /// The pairing code to display (QR / copy-paste).
    pub code: PairingCode,
}

/// Begin pairing on an existing device: generate an ephemeral keypair, open the
/// relay `offer1`, and return the [`PairingCode`] to display.
///
/// `connection` is the relay/server URL embedded in the code for reference.
///
/// # Errors
///
/// Returns an error if the relay offer cannot be created.
pub async fn initiate_pairing<T: RawHttpSender>(
    client: &ServerSyncClient<T>,
    connection: &str,
) -> Result<Initiation, PairingError> {
    let init = initiate_onboarding(connection);

    // `offer1`'s payload is opaque to the server; the initiator public key is
    // also shipped in the QR, but the relay requires a non-empty body.
    let offer1_data = init.qr_payload.public_key.clone().into_bytes();
    let offer = client.create_offer(&offer1_data).await?;

    let code = PairingCode {
        offer_id: offer.offer_id,
        public_key: init.qr_payload.public_key,
        connection: init.qr_payload.connection,
        verification_code: init.qr_payload.verification_code,
    };

    Ok(Initiation {
        secret: init.secret,
        code,
    })
}

/// The joiner's public key + return offer, awaited by the initiator.
pub struct JoinerHelloReceived {
    public_key: String,
    response_offer_id: String,
}

/// Poll `offer1` for the joiner's hello. `Ok(None)` means the joiner has not
/// responded yet — the caller should back off and retry.
///
/// # Errors
///
/// Returns an error if the relay call fails or the hello cannot be decoded.
pub async fn poll_joiner_hello<T: RawHttpSender>(
    client: &ServerSyncClient<T>,
    offer_id: &str,
) -> Result<Option<JoinerHelloReceived>, PairingError> {
    match client.get_offer_response(offer_id).await {
        Ok(bytes) => {
            let hello: JoinerHello = serde_json::from_slice(&bytes)
                .map_err(|e| PairingError::InvalidPayload(e.to_string()))?;
            Ok(Some(JoinerHelloReceived {
                public_key: hello.public_key,
                response_offer_id: hello.response_offer_id,
            }))
        }
        Err(ServerSyncError::Http { status: 404, .. }) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Complete the exchange on the initiating device: derive the shared secret from
/// the joiner's public key, encrypt the vault key under it, and post the
/// ciphertext to the joiner's return offer.
///
/// # Errors
///
/// Returns an error if key derivation/encryption fails or the relay post fails.
pub async fn deliver_vault_key<T: RawHttpSender>(
    client: &ServerSyncClient<T>,
    initiation: Initiation,
    hello: &JoinerHelloReceived,
    vault_key: &[u8; 32],
) -> Result<(), PairingError> {
    let shared = complete_onboarding(initiation.secret, &hello.public_key)
        .map_err(PairingError::KeyExchange)?;
    let ciphertext = encrypt_vault_key(&shared, vault_key).map_err(PairingError::KeyExchange)?;
    client
        .post_offer_response(&hello.response_offer_id, &ciphertext)
        .await?;
    Ok(())
}

/// State the joining device keeps between [`respond_pairing`] and
/// [`poll_vault_key`]. Holds the shared secret, zeroized on drop.
pub struct JoinerSession {
    shared_secret: [u8; 32],
    response_offer_id: String,
    /// Six-digit verification code to compare against the initiating device.
    pub verification_code: String,
}

impl Drop for JoinerSession {
    fn drop(&mut self) {
        self.shared_secret.zeroize();
    }
}

impl JoinerSession {
    /// The joiner's return-mailbox relay offer id (`offer2`) the initiator posts
    /// the encrypted vault key to.
    #[must_use]
    pub fn response_offer_id(&self) -> &str {
        &self.response_offer_id
    }
}

/// Respond to a pairing code on the new device: derive the shared secret, open a
/// return-mailbox `offer2`, and post `{pubB, offer2}` as the response to
/// `offer1`.
///
/// The returned [`JoinerSession`] carries the verification code to confirm with
/// the initiating device before trusting the exchange.
///
/// # Errors
///
/// Returns an error if the key exchange or any relay call fails.
pub async fn respond_pairing<T: RawHttpSender>(
    client: &ServerSyncClient<T>,
    code: &PairingCode,
) -> Result<JoinerSession, PairingError> {
    let qr = QrPayload {
        public_key: code.public_key.clone(),
        connection: code.connection.clone(),
        verification_code: code.verification_code.clone(),
    };
    let resp = respond_to_onboarding(&qr).map_err(PairingError::KeyExchange)?;

    // The joiner's own offer is the mailbox the initiator posts the encrypted
    // vault key to. Its body is opaque; reuse the joiner public key as content.
    let offer2 = client.create_offer(resp.public_key_b64.as_bytes()).await?;

    let hello = JoinerHello {
        public_key: resp.public_key_b64,
        response_offer_id: offer2.offer_id.clone(),
    };
    let hello_bytes =
        serde_json::to_vec(&hello).map_err(|e| PairingError::InvalidPayload(e.to_string()))?;
    client
        .post_offer_response(&code.offer_id, &hello_bytes)
        .await?;

    Ok(JoinerSession {
        shared_secret: resp.shared_secret,
        response_offer_id: offer2.offer_id,
        verification_code: resp.verification_code,
    })
}

/// Poll `offer2` for the encrypted vault key and decrypt it. `Ok(None)` means
/// the initiator has not delivered it yet — the caller should back off and retry.
///
/// # Errors
///
/// Returns an error if the relay call fails or decryption fails (wrong shared
/// secret / corrupted payload).
pub async fn poll_vault_key<T: RawHttpSender>(
    client: &ServerSyncClient<T>,
    session: &JoinerSession,
) -> Result<Option<[u8; 32]>, PairingError> {
    match client.get_offer_response(&session.response_offer_id).await {
        Ok(ciphertext) => {
            let key = decrypt_vault_key(&session.shared_secret, &ciphertext)
                .map_err(PairingError::KeyExchange)?;
            Ok(Some(key))
        }
        Err(ServerSyncError::Http { status: 404, .. }) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_code_round_trips() {
        let code = PairingCode {
            offer_id: "0191f0aa-1234-7abc-9def-0123456789ab".into(),
            public_key: "AAAABBBBCCCCDDDDEEEEFFFF00001111".into(),
            connection: "https://sync.example.com".into(),
            verification_code: "042195".into(),
        };
        let token = code.encode();
        let decoded = PairingCode::decode(&token).expect("decode");
        assert_eq!(decoded, code);
    }

    #[test]
    fn pairing_code_decode_rejects_garbage() {
        // Not valid base64 of any JSON object.
        assert!(PairingCode::decode("!!!not-base64!!!").is_err());
        // Valid base64 but not a PairingCode.
        let not_a_code = base64_encode(b"{\"nope\":true}");
        assert!(PairingCode::decode(&not_a_code).is_err());
    }

    #[test]
    fn pairing_code_token_is_whitespace_tolerant() {
        let code = PairingCode {
            offer_id: "id".into(),
            public_key: "pk".into(),
            connection: "conn".into(),
            verification_code: "000000".into(),
        };
        let token = code.encode();
        let padded = format!("  {token}\n");
        assert_eq!(PairingCode::decode(&padded).expect("decode"), code);
    }
}
