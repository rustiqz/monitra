//! The one client implementation (DESIGN.md §4 `tui`, ADR-009) — an
//! HTTP/WS client against `monitra-backend`'s API. Never touches
//! `monitra-storage`/`monitra-provider`, in any mode (§11.4).
//!
//! DTOs here are local, not shared with `monitra-backend`'s (the DAG
//! forbids depending on it) — mirrors `backend`'s own "wire format
//! independent of the internal model" stance (`crates/backend/src/monitors.rs`),
//! just from the client side of the same wire shape.

use monitra_models::{Agent, Monitor, MonitorKind, MonitorStatus};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("tui: request to {url} failed: {source}")]
    Request { url: String, source: reqwest::Error },
    #[error("tui: {url} returned {status}")]
    Status {
        url: String,
        status: reqwest::StatusCode,
    },
    #[error("tui: failed to decode response from {url}: {source}")]
    Decode { url: String, source: reqwest::Error },
}

#[derive(Debug, Clone, Deserialize)]
pub struct MonitorDto {
    pub id: u64,
    pub name: String,
    pub target: String,
    pub kind: MonitorKind,
    pub interval_secs: u64,
    pub status: MonitorStatus,
    pub agent_id: Option<u64>,
}

impl From<MonitorDto> for Monitor {
    fn from(dto: MonitorDto) -> Self {
        Monitor {
            id: dto.id,
            name: dto.name,
            target: dto.target,
            kind: dto.kind,
            interval_secs: dto.interval_secs,
            status: dto.status,
            agent_id: dto.agent_id,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentDto {
    pub id: u64,
    pub name: String,
    pub last_heartbeat_at: u64,
    pub scope: String,
}

impl From<AgentDto> for Agent {
    fn from(dto: AgentDto) -> Self {
        Agent {
            id: dto.id,
            name: dto.name,
            last_heartbeat_at: dto.last_heartbeat_at,
            scope: dto.scope,
            // The push token is never sent over `GET /agents` (only once,
            // at `register` — `crates/backend/src/agents.rs`); the TUI never
            // needs it, so it's left blank rather than made `Option`, which
            // would ripple the field's meaning into every other consumer of
            // `monitra_models::Agent`.
            token: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlertEventDto {
    pub id: u64,
    pub monitor_id: u64,
    pub transitioned_to: MonitorStatus,
    pub occurred_at: u64,
    pub sinks_attempted: String,
    pub delivery_outcome: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CheckResultDto {
    pub checked_at: u64,
    pub success: bool,
    pub latency_ms: u64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub store: StoreHealth,
    pub cache: CacheHealth,
    pub notifier: NotifierHealth,
    pub k8s_clusters: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StoreHealth {
    pub name: String,
    pub reachable: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheHealth {
    pub name: String,
    pub degraded: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotifierHealth {
    pub name: String,
    pub queue_len: usize,
}

/// A `monitra-backend` API client. Cheap to clone — `reqwest::Client` is
/// itself an `Arc` internally, and the base URL/token are plain owned data.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl Client {
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            token,
        }
    }

    /// The `ws://` (or `wss://`) form of the base URL's `/ws` route, for
    /// [`crate::data`]'s live-update subscriber.
    pub fn ws_url(&self) -> String {
        let ws_base = self
            .base_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        format!("{ws_base}/ws")
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// What the header shows as "connected to" — never includes the
    /// token.
    pub fn base_url_display(&self) -> String {
        self.base_url.clone()
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ClientError> {
        let url = format!("{}{path}", self.base_url);
        let response = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|source| ClientError::Request {
                url: url.clone(),
                source,
            })?;
        if !response.status().is_success() {
            return Err(ClientError::Status {
                url,
                status: response.status(),
            });
        }
        response
            .json::<T>()
            .await
            .map_err(|source| ClientError::Decode { url, source })
    }

    pub async fn list_monitors(&self) -> Result<Vec<MonitorDto>, ClientError> {
        self.get("/monitors").await
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentDto>, ClientError> {
        self.get("/agents").await
    }

    pub async fn list_alerts(&self) -> Result<Vec<AlertEventDto>, ClientError> {
        self.get("/alerts").await
    }

    pub async fn monitor_history(&self, id: u64) -> Result<Vec<CheckResultDto>, ClientError> {
        self.get(&format!("/monitors/{id}/history")).await
    }

    /// `/health` needs no bearer token on the wire (P6 — it must answer even
    /// with a lost token), but this client sends one anyway since it's
    /// cheap and the server ignores it here; kept as a plain `get` for
    /// consistency with every other call rather than a special case.
    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        self.get("/health").await
    }
}
