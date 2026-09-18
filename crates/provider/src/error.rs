//! `provider`'s error type. Every variant names the component and the
//! specific thing that failed (CLAUDE.md "Errors name their component") —
//! never a bare "error: denied".

use std::path::PathBuf;

use crate::category::ProviderCategory;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider: config: failed to read {path}: {source}")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("provider: config: failed to write {path}: {source}")]
    ConfigWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("provider: config: {path} is not valid TOML: {source}")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("provider: config: failed to serialize config for {path}: {source}")]
    ConfigSerialize {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },

    #[error(
        "provider: registry: {scheme} is not a recognized provider scheme \
         (known: postgres, redis, slack, webhook)"
    )]
    UnknownScheme { scheme: String },

    #[error("provider: {category}: unavailable: {detail}")]
    Unavailable {
        category: ProviderCategory,
        detail: String,
    },

    /// A provider was reachable but an individual operation on it failed
    /// (a bad query, a constraint violation, a corrupt row) — distinct from
    /// `Unavailable`, which is the §4.1 "could not be reached at all" case
    /// that drives fail-fast for `Store`.
    #[error("provider: {category}: operation failed: {detail}")]
    Operation {
        category: ProviderCategory,
        detail: String,
    },
}
