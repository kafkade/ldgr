//! Sync engine: event generation, batch encryption, conflict resolution,
//! snapshot compaction, device onboarding, and transport types.
//!
//! Pure computation — no networking. Platform code handles transport
//! (blob store upload/download, server relay, QR display/scan).

pub mod conflicts;
pub mod events;
pub mod onboarding;
pub mod snapshot;
pub mod transport;

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
