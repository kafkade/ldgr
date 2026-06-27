//! Sync engine: event generation, batch encryption, conflict resolution,
//! snapshot compaction, device onboarding, and transport types.
//!
//! Pure computation — no networking. Platform code handles transport
//! (blob store upload/download, server relay, QR display/scan).

pub mod conflicts;
pub mod events;
pub mod onboarding;
pub mod payload;
pub mod snapshot;
pub mod transport;

/// Server sync protocol (SRP-6a client + endpoint types). Requires the `sync`
/// feature, which pulls in big-integer arithmetic for SRP.
#[cfg(feature = "sync")]
pub mod server;

/// Batch-blob compose/apply pipeline (pending events ↔ encrypted blob).
/// Requires the `sqlite` feature because it reads and writes the canonical
/// `SQLite` vault via a passed-in `Connection`.
#[cfg(feature = "sqlite")]
pub mod pipeline;

pub use conflicts::*;
pub use events::*;
pub use onboarding::{
    OnboardingInitiation, OnboardingResponse, QrPayload, complete_onboarding, decrypt_vault_key,
    encrypt_vault_key, initiate_onboarding, respond_to_onboarding,
};
pub use snapshot::*;
pub use transport::{
    BatchRef, BlobEntry, BlobPath, BlobPrefix, DeviceInfo, ListResult, PutResult, RemoteBatchMeta,
    RemoteSnapshotMeta, RetryPolicy, SyncCheckpoint, TransportConfig, TransportErrorKind,
    TransportProvider,
};
