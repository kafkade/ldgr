//! Sync engine: event generation, batch encryption, and conflict resolution.
//!
//! Pure computation — no networking. Platform code handles transport
//! (blob store upload/download, server relay).

pub mod conflicts;
pub mod events;

pub use conflicts::*;
pub use events::*;
