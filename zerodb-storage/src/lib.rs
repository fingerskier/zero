use thiserror::Error;
use zerodb_core::types::OpId;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("operation not found: {0:?}")]
    NotFound(OpId),
    #[error("storage I/O error: {0}")]
    Io(String),
}

/// Trait that all storage backends must implement.
///
/// Experimental pre-M0 scaffold — not normative. Will be replaced by
/// SPEC §7 OplogStore/StateStore (+ C8/M0e atomicity) after composite M0.
#[async_trait::async_trait]
pub trait StorageAdapter: Send + Sync {
    /// Store a raw operation (serialized bytes) by its OpId.
    async fn put_op(&self, id: OpId, data: &[u8]) -> Result<(), StorageError>;

    /// Retrieve a raw operation by OpId.
    async fn get_op(&self, id: &OpId) -> Result<Vec<u8>, StorageError>;

    /// Check if an operation exists.
    async fn has_op(&self, id: &OpId) -> Result<bool, StorageError>;
}
