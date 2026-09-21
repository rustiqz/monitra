//! HTTP push client (DESIGN.md §4 `agent`, §11.10) — `POST
//! /agents/{id}/ingest`, bearer-authenticated. Mirrors the wire shape
//! `monitra-backend`'s `ingest.rs` deserializes (`PushedOutcomeDto`)
//! without sharing a type — `monitra-agent` may never depend on
//! `monitra-backend` or `monitra-engine` (DAG, ADR-008).
//!
//! Also owns `GET /agents/{id}/assignments` (ADR-011, Phase 11) — the pull
//! side of regional probing. Unlike the ingest wire shape, this DTO reuses
//! `monitra_models::MonitorKind` directly rather than mirroring it: the DAG
//! already lets `monitra-agent` depend on `monitra-models`, so there is no
//! boundary reason to duplicate it, only for the ingest outcome shape,
//! which historically also carried the now-removed `CheckOutcome` split.

use std::time::Duration;

use monitra_models::MonitorKind;
use monitra_probe::{NetworkProbeKind, ProbeOutcome};
use serde::{Deserialize, Serialize};

use crate::error::AgentError;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum WireOutcome {
    Success { latency_ms: u64 },
    Failure { message: String },
    Unavailable { message: String },
}

impl From<&ProbeOutcome> for WireOutcome {
    fn from(outcome: &ProbeOutcome) -> Self {
        match outcome {
            ProbeOutcome::Success { latency_ms } => WireOutcome::Success {
                latency_ms: *latency_ms,
            },
            ProbeOutcome::Failure { message } => WireOutcome::Failure {
                message: message.clone(),
            },
            ProbeOutcome::Unavailable { message } => WireOutcome::Unavailable {
                message: message.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct WireResult {
    monitor_id: u64,
    #[serde(flatten)]
    outcome: WireOutcome,
}

#[derive(Debug, Clone, Serialize)]
struct IngestBody {
    results: Vec<WireResult>,
}

/// One check's result, paired with the `Monitor` it belongs to — the
/// agent's config ties the two together (§11.10: `monitor_id` is
/// cross-checked server-side against `Monitor.agent_id` before being
/// forwarded, so a wrong id here is dropped there, not trusted blindly).
#[derive(Debug, Clone)]
pub struct PendingResult {
    pub monitor_id: u64,
    pub outcome: ProbeOutcome,
}

/// One network-probe monitor this agent has been assigned to check
/// (ADR-011, Phase 11) — what `GET /agents/{id}/assignments` returns.
/// `kind` is the full `monitra_models::MonitorKind` as `monitra-backend`
/// sends it (it doesn't know or care which subset is probe-able); this
/// side narrows it to a [`NetworkProbeKind`] before probing, same as the
/// central scheduler does.
#[derive(Debug, Clone, Deserialize)]
pub struct AssignmentDto {
    pub monitor_id: u64,
    pub target: String,
    pub kind: MonitorKind,
}

impl AssignmentDto {
    /// `None` for a kind that isn't a network probe — shouldn't happen
    /// (the engine only ever enqueues `Http`/`Tcp`/`Icmp` monitors here),
    /// but a wire payload is never trusted to already satisfy an invariant
    /// this side depends on (P1).
    pub fn probe_kind(&self) -> Option<NetworkProbeKind> {
        NetworkProbeKind::try_from(self.kind).ok()
    }
}

#[derive(Debug, Deserialize)]
struct AssignmentsBody {
    assignments: Vec<AssignmentDto>,
}

pub struct IngestClient {
    http: reqwest::Client,
    base_url: String,
    agent_id: u64,
    token: String,
}

impl IngestClient {
    pub fn new(base_url: String, agent_id: u64, token: String) -> Result<Self, AgentError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|source| AgentError::ClientBuild { source })?;
        Ok(Self {
            http,
            base_url,
            agent_id,
            token,
        })
    }

    /// Pushes a batch in a single POST (§11.10's batching philosophy — one
    /// call per cycle, not one per check). A push happening at all updates
    /// the agent's heartbeat server-side, even when `results` is empty, so
    /// liveness stays accurate independently of whether any check produced
    /// something new this cycle.
    pub async fn push(&self, results: &[PendingResult]) -> Result<(), AgentError> {
        let body = IngestBody {
            results: results
                .iter()
                .map(|pending| WireResult {
                    monitor_id: pending.monitor_id,
                    outcome: (&pending.outcome).into(),
                })
                .collect(),
        };
        let url = format!("{}/agents/{}/ingest", self.base_url, self.agent_id);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|source| AgentError::Push {
                url: url.clone(),
                source,
            })?;

        if !response.status().is_success() {
            return Err(AgentError::PushRejected {
                url,
                status: response.status().as_u16(),
            });
        }
        Ok(())
    }

    /// Pulls whatever regional probes are currently queued for this agent
    /// (ADR-011, Phase 11) — the engine already decided these are due;
    /// this is a pull, not a subscription, so an empty list is the normal
    /// "nothing due right now" case, not an error.
    pub async fn fetch_assignments(&self) -> Result<Vec<AssignmentDto>, AgentError> {
        let url = format!("{}/agents/{}/assignments", self.base_url, self.agent_id);
        let response = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|source| AgentError::FetchAssignments {
                url: url.clone(),
                source,
            })?;

        if !response.status().is_success() {
            return Err(AgentError::FetchAssignmentsRejected {
                url,
                status: response.status().as_u16(),
            });
        }

        let body: AssignmentsBody = response
            .json()
            .await
            .map_err(|source| AgentError::FetchAssignmentsDecode { url, source })?;
        Ok(body.assignments)
    }
}
