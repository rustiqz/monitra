//! `AgentError` (DESIGN.md §7.1 — per-crate error enum, named component).

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent: failed to read token file {path}: {source}")]
    TokenFileRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("agent: token file {path} is empty")]
    TokenFileEmpty { path: PathBuf },
    #[error("agent: failed to read check config {path}: {source}")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("agent: failed to parse check config {path}: {source}")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("agent: failed to build HTTP client: {source}")]
    ClientBuild {
        #[source]
        source: reqwest::Error,
    },
    #[error("agent: push to {url} failed: {source}")]
    Push {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("agent: push to {url} was rejected: HTTP {status}")]
    PushRejected { url: String, status: u16 },
    #[error("agent: fetching assignments from {url} failed: {source}")]
    FetchAssignments {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("agent: fetching assignments from {url} was rejected: HTTP {status}")]
    FetchAssignmentsRejected { url: String, status: u16 },
    #[error("agent: failed to decode assignments response from {url}: {source}")]
    FetchAssignmentsDecode {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("agent: neither --token nor --token-file was given")]
    MissingToken,
}
