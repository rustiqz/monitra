//! `POST /agents/{id}/ingest` — the authenticated agent-push endpoint
//! (DESIGN.md §4 `backend`, ADR-008, §11.10). Auth is `auth::require_agent_token`
//! (a route layer, checked before this handler runs).
//!
//! Batched (§6.2's batching philosophy, and an agent performs several local
//! checks per cycle): one POST carries every result that agent produced
//! this cycle, plus updates its heartbeat once — a push happening at all is
//! itself the liveness signal, independent of whether any individual
//! result in the batch turns out to be misattributed.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use monitra_engine::ProbeOutcome;
use serde::Deserialize;

use crate::AppState;
use crate::error::ApiError;
use crate::service::now_unix_secs;

/// Mirrors [`ProbeOutcome`] on the wire (Phase 8, §11.10 addendum) — a
/// `HostAgentCheck` the agent could not itself perform (permission denied,
/// `systemctl`/dbus unreachable) tags `unavailable`, not `failure`, so it
/// never renders as target-down (P1, §11.3's same honesty requirement
/// extended to the push path).
#[derive(Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PushedOutcomeDto {
    Success { latency_ms: u64 },
    Failure { message: String },
    Unavailable { message: String },
}

impl From<PushedOutcomeDto> for ProbeOutcome {
    fn from(dto: PushedOutcomeDto) -> Self {
        match dto {
            PushedOutcomeDto::Success { latency_ms } => ProbeOutcome::Success { latency_ms },
            PushedOutcomeDto::Failure { message } => ProbeOutcome::Failure { message },
            PushedOutcomeDto::Unavailable { message } => ProbeOutcome::Unavailable { message },
        }
    }
}

#[derive(Deserialize)]
pub struct PushedResultDto {
    pub monitor_id: u64,
    #[serde(flatten)]
    pub outcome: PushedOutcomeDto,
}

#[derive(Deserialize)]
pub struct IngestRequest {
    pub results: Vec<PushedResultDto>,
}

pub async fn push(
    State(state): State<AppState>,
    Path(agent_id): Path<u64>,
    Json(body): Json<IngestRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .heartbeat_agent(agent_id, now_unix_secs())
        .await?;

    for result in body.results {
        let monitor_id = result.monitor_id;
        match state.store.get_monitor(monitor_id).await {
            Ok(Some(monitor)) if monitor.agent_id == Some(agent_id) => {
                state.ingest.submit(monitra_engine::PushedResult {
                    monitor_id,
                    outcome: result.outcome.into(),
                });
            }
            Ok(Some(_)) => {
                tracing::warn!(
                    agent_id,
                    monitor_id,
                    "backend: ingest: monitor does not belong to this agent, dropping result"
                );
            }
            Ok(None) => {
                tracing::warn!(
                    agent_id,
                    monitor_id,
                    "backend: ingest: unknown monitor, dropping result"
                );
            }
            Err(source) => {
                tracing::warn!(
                    agent_id,
                    monitor_id,
                    error = %source,
                    "backend: ingest: failed to look up monitor, dropping result"
                );
            }
        }
    }

    Ok(StatusCode::ACCEPTED)
}
