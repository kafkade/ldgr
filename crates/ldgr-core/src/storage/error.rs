//! Storage layer errors.

use thiserror::Error;

/// Errors that can occur during storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    /// The requested entity was not found (or is soft-deleted).
    #[error("not found: {0}")]
    NotFound(String),

    /// A write conflict: version mismatch or entity was deleted.
    #[error("conflict: {0}")]
    Conflict(String),

    /// Constraint violation (duplicate name, FK violation, etc.).
    #[error("constraint violation: {0}")]
    ConstraintViolation(String),

    /// Invalid input data.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Underlying `SQLite` error.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}
