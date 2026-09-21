//! `GET /agents/{id}/assignments` — the pull side of regional latency
//! probing (DESIGN.md §4 `backend`/`engine`, ADR-011, Phase 11). Auth is
//! `auth::require_agent_token`, the same middleware `ingest.rs` uses, scoped
//! to the one agent named by `{id}` (a route layer, checked before this
//! handler runs).
//!
//! Read-only from `backend`'s point of view: it drains whatever the
//! engine's scheduler has already decided is due for this agent
//! ([`monitra_engine::AssignmentHandle`]) and hands it back as JSON. The
//! agent probes it and reports the result through the existing
//! `POST /agents/{id}/ingest` — this route never sees a result, only the
//! assignment.

use axum::Json;
use axum::extract::{Path, State};
use monitra_models::MonitorKind;
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct AssignmentDto {
    pub monitor_id: u64,
    pub target: String,
    pub kind: MonitorKind,
}

impl From<monitra_engine::Assignment> for AssignmentDto {
    fn from(assignment: monitra_engine::Assignment) -> Self {
        AssignmentDto {
            monitor_id: assignment.monitor_id,
            target: assignment.target,
            kind: assignment.kind,
        }
    }
}

#[derive(Serialize)]
pub struct AssignmentsResponse {
    pub assignments: Vec<AssignmentDto>,
}

/// Draining, not peeking — once returned, an assignment isn't handed to
/// this agent again on the next poll (`AssignmentHandle::drain`). An empty
/// list is the ordinary "nothing due right now" case, not an error — this
/// is a pure in-memory operation that cannot itself fail, unlike every
/// other handler here that goes through `state.store`.
pub async fn list(
    State(state): State<AppState>,
    Path(agent_id): Path<u64>,
) -> Json<AssignmentsResponse> {
    let assignments = state
        .assignments
        .drain(agent_id)
        .into_iter()
        .map(Into::into)
        .collect();
    Json(AssignmentsResponse { assignments })
}
