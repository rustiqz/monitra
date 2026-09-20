//! HTTP push client (DESIGN.md §4 `agent`, §11.10) — `POST
//! /agents/{id}/ingest`, bearer-authenticated. Mirrors the wire shape
//! `monitra-backend`'s `ingest.rs` deserializes (`PushedOutcomeDto`)
//! without sharing a type — `monitra-agent` may never depend on
//! `monitra-backend` or `monitra-engine` (DAG, ADR-008).

use std::time::Duration;

use serde::Serialize;

use crate::checks::CheckOutcome;
use crate::error::AgentError;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum WireOutcome {
    Success { latency_ms: u64 },
    Failure { message: String },
    Unavailable { message: String },
}

impl From<&CheckOutcome> for WireOutcome {
    fn from(outcome: &CheckOutcome) -> Self {
        match outcome {
            CheckOutcome::Success { latency_ms } => WireOutcome::Success {
                latency_ms: *latency_ms,
            },
            CheckOutcome::Failure { message } => WireOutcome::Failure {
                message: message.clone(),
            },
            CheckOutcome::Unavailable { message } => WireOutcome::Unavailable {
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
    pub outcome: CheckOutcome,
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
}
