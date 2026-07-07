//! Sync engine: event generation, batch encryption, conflict resolution,
//! snapshot compaction, device onboarding, and transport types.
//!
//! Pure computation — no networking. Platform code handles transport
//! (blob store upload/download, server relay, QR display/scan).

pub mod conflicts;
pub mod events;
pub mod framing;
pub mod onboarding;
pub mod payload;
pub mod snapshot;
pub mod transport;

/// Server sync protocol (SRP-6a client + endpoint types). Requires the `sync`
/// feature, which pulls in big-integer arithmetic for SRP.
#[cfg(feature = "sync")]
pub mod server;

/// Relay-backed device pairing orchestration (two-offer X25519 handshake).
/// Requires the `sync` feature because it drives the server key-exchange relay.
#[cfg(feature = "sync")]
pub mod pairing;

/// Batch-blob compose/apply pipeline (pending events ↔ encrypted blob).
/// Requires the `sqlite` feature because it reads and writes the canonical
/// `SQLite` vault via a passed-in `Connection`.
#[cfg(feature = "sqlite")]
pub mod pipeline;

pub use conflicts::*;
pub use events::*;
pub use framing::{
    FramingError, open_batch, open_batch_with_session_key, seal_batch, seal_batch_with_session_key,
};
pub use onboarding::{
    OnboardingInitiation, OnboardingResponse, QrPayload, complete_onboarding, decrypt_vault_key,
    encrypt_vault_key, initiate_onboarding, respond_to_onboarding,
};

#[cfg(feature = "sync")]
pub use pairing::{
    Initiation, JoinerHelloReceived, JoinerSession, PairingCode, PairingError, deliver_vault_key,
    initiate_pairing, poll_joiner_hello, poll_vault_key, respond_pairing,
};
pub use snapshot::*;
pub use transport::{
    BatchRef, BlobEntry, BlobPath, BlobPrefix, DeviceInfo, ListResult, PutResult, RemoteBatchMeta,
    RemoteSnapshotMeta, RetryPolicy, SyncCheckpoint, TransportConfig, TransportErrorKind,
    TransportProvider,
};
