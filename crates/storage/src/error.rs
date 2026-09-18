//! `storage`'s own error vocabulary (CLAUDE.md "errors name their
//! component"). Converted into `monitra_provider::ProviderError` at the
//! `Store` trait boundary — nothing outside this crate ever sees a
//! `StorageError` or a raw `rusqlite::Error`.

use monitra_provider::{ProviderCategory, ProviderError};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage: failed to open {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: rusqlite::Error,
    },

    #[error("storage: migration {version} failed: {source}")]
    MigrationFailed {
        version: &'static str,
        #[source]
        source: rusqlite::Error,
    },

    #[error("storage: query failed: {source}")]
    QueryFailed {
        #[source]
        source: rusqlite::Error,
    },

    /// A stored enum column (`kind`, `status`, `transitioned_to`) held text
    /// that doesn't match any known variant — a corrupt row, not a query
    /// failure, distinct so it's diagnosable at 3 a.m. (P6) rather than
    /// reading as a generic SQL error.
    #[error("storage: corrupt row: {detail}")]
    CorruptRow { detail: String },

    #[error("storage: {entity} {id} not found")]
    NotFound { entity: &'static str, id: u64 },

    #[error("storage: background task failed: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

impl From<StorageError> for ProviderError {
    fn from(error: StorageError) -> Self {
        ProviderError::Operation {
            category: ProviderCategory::Store,
            detail: error.to_string(),
        }
    }
}
