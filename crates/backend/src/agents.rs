//! `/agents` handlers and DTOs (DESIGN.md §4 `backend`, ADR-008).
//!
//! Registration/list/remove only — the authenticated push-ingest endpoint
//! and heartbeat handling are Phase 7/8 (§4 `agent`, roadmap).

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
}

impl From<monitra_models::Agent> for AgentDto {
    fn from(agent: monitra_models::Agent) -> Self {
        AgentDto {
            id: agent.id,
            name: agent.name,
            last_heartbeat_at: agent.last_heartbeat_at,
            scope: agent.scope,
        }
    }
}

#[derive(Deserialize)]
pub struct RegisterAgentRequest {
    pub name: String,
    pub scope: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterAgentRequest>,
) -> Result<(StatusCode, Json<AgentDto>), ApiError> {
    let agent = service::register_agent(state.store.as_ref(), body.name, body.scope).await?;
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
