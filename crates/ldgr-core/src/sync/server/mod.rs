//! Server sync protocol: SRP-6a client primitives, serde request/response
//! types, and a transport-agnostic [`ServerSyncClient`].
//!
//! These are the pure, no-I/O building blocks every platform client
//! (CLI/reqwest, iOS/URLSession, web/fetch) reuses to talk to `ldgr-server`.
//! Actual HTTP is performed by platform code via the injected
//! [`RawHttpSender`] callback.

pub mod client;
pub mod protocol;
pub mod srp;

pub use client::{
    HttpMethod, RawHttpSender, RawRequest, RawResponse, ServerSyncClient, ServerSyncError,
};
pub use protocol::{
    BlobEntry, CreateOfferRequest, CreateOfferResponse, CreateVaultRequest, DeviceResponse,
    ErrorResponse, GetResponseResponse, HexError, ListBatchesQuery, ListBlobsResponse,
    ListSnapshotsQuery, LoginInitRequest, LoginInitResponse, LoginVerifyRequest,
    LoginVerifyResponse, OfferResponse, PostResponseRequest, PutBlobResponse, RegisterRequest,
    RegisterResponse, VaultResponse, hex_decode, hex_encode,
};
pub use srp::{
    ClientLogin, ClientSession, RegistrationVerifier, SrpError, register, register_with_salt,
};
