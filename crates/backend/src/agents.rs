//! `/agents` handlers and DTOs (DESIGN.md §4 `backend`, ADR-008).
//!
//! Registration/list/remove; the authenticated push-ingest endpoint lives in
//! `ingest.rs` (Phase 7).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::ApiError;
use crate::service;

#[derive(Serialize)]
pub struct AgentDto {
    pub id: u64,
    pub name: String,
    pub last_heartbeat_at: u64,
    pub scope: String,
    /// Vantage point this agent probes from (ADR-011) — `None` when
    /// unset, never guessed or defaulted; excludes it from regional views.
    pub region: Option<String>,
}

impl From<monitra_models::Agent> for AgentDto {
    fn from(agent: monitra_models::Agent) -> Self {
        AgentDto {
            id: agent.id,
            name: agent.name,
            last_heartbeat_at: agent.last_heartbeat_at,
            scope: agent.scope,
            region: agent.region,
        }
    }
}

#[derive(Deserialize)]
pub struct RegisterAgentRequest {
    pub name: String,
    pub scope: String,
    /// Optional (ADR-011) — omitting it, including on a repeat
    /// registration, means "no region," full overwrite (§4 `backend`).
    #[serde(default)]
    pub region: Option<String>,
}

/// Includes the push token — unlike [`AgentDto`], and only here: this is
/// the one moment it's shown (§11.10, mirroring the human API token's
/// "generated once, never re-shown" pattern, §11.11). A repeat `register`
/// under the same name issues (and shows) a fresh one, rotating the old.
#[derive(Serialize)]
pub struct RegisterAgentResponse {
    pub id: u64,
    pub name: String,
    pub scope: String,
    pub token: String,
    pub region: Option<String>,
}

impl From<monitra_models::Agent> for RegisterAgentResponse {
    fn from(agent: monitra_models::Agent) -> Self {
        RegisterAgentResponse {
            id: agent.id,
            name: agent.name,
            scope: agent.scope,
            token: agent.token,
            region: agent.region,
        }
    }
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterAgentRequest>,
) -> Result<(StatusCode, Json<RegisterAgentResponse>), ApiError> {
    let agent =
        service::register_agent(state.store.as_ref(), body.name, body.scope, body.region).await?;
    Ok((StatusCode::CREATED, Json(agent.into())))
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<AgentDto>>, ApiError> {
    let agents = service::list_agents(state.store.as_ref()).await?;
    Ok(Json(agents.into_iter().map(Into::into).collect()))
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    service::remove_agent(state.store.as_ref(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}
