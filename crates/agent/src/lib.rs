//! Remote collection and local host checks (DESIGN.md §4 `agent`, ADR-008).
//!
//! Owns the `monitra agent run` mode: local host checks (systemd unit
//! status, disk space, process liveness — no black-box network equivalent
//! exists for these), the push loop itself (retry-on-next-cycle, a bounded
//! local buffer for when the backend is unreachable, §7.3), and token
//! handling. Reports; does not interpret — `monitra-engine` decides what a
//! pushed result means once it lands via `monitra-backend`'s ingest
//! endpoint (§4 `backend`).
//!
//! Never discovers its monitors from the backend — there is no
//! "list my monitors" endpoint, deliberately. The operator ties a local
//! check to a `Monitor` row by `monitor_id` in the `--config` file
//! (`config.rs`); `engine`'s scheduler already never dispatches
//! `HostAgentCheck` monitors itself, so this crate is the only producer of
//! their results.
//!
//! K8s-fallback push (ADR-008's second mechanism) is deliberately out of
//! scope for Phase 8 — `monitra-agent` may never depend on
//! `monitra-provider` (DAG), so it cannot reuse `collector-kubernetes` and
//! would need its own from-scratch Kubernetes-API polling code. Tracked as
//! a follow-up (DESIGN.md §10 Phase 8 row).

pub mod buffer;
pub mod checks;
pub mod client;
pub mod config;
pub mod error;

use std::path::PathBuf;
use std::time::Duration;

use buffer::PushBuffer;
use client::{IngestClient, PendingResult};
use config::CheckDefinition;
use error::AgentError;

/// What `main.rs` builds from `monitra agent run`'s CLI arguments.
pub struct RunConfig {
    /// Human label, for logging only — the push path is keyed by `agent_id`.
    pub name: String,
    pub scope: String,
    pub backend_url: String,
    pub agent_id: u64,
    pub token: Option<String>,
    pub token_file: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    /// Bounded local buffer capacity (§7.3) — how many results survive a
    /// backend outage before the oldest are dropped-and-logged.
    pub buffer_capacity: usize,
}

impl RunConfig {
    fn resolve_token(&self) -> Result<String, AgentError> {
        if let Some(token) = &self.token {
            return Ok(token.clone());
        }
        if let Some(path) = &self.token_file {
            let contents =
                std::fs::read_to_string(path).map_err(|source| AgentError::TokenFileRead {
                    path: path.clone(),
                    source,
                })?;
            let trimmed = contents.trim();
            if trimmed.is_empty() {
                return Err(AgentError::TokenFileEmpty { path: path.clone() });
            }
            return Ok(trimmed.to_string());
        }
        Err(AgentError::MissingToken)
    }

    fn load_checks(&self) -> Result<config::CheckConfig, AgentError> {
        match &self.config_path {
            Some(path) => config::load(path),
            // No config at all is valid, not an error — an agent with
            // nothing configured still registers its heartbeat (§11.7's
            // "never mandatory" spirit applied here: there is no honest
            // embedded default for *what* to check, so the honest default
            // is "nothing" rather than guessing).
            None => Ok(config::CheckConfig::default()),
        }
    }
}

/// Runs the agent process until a shutdown signal arrives. Local checks and
/// the push loop share one cycle deliberately (not two independent tasks):
/// a slow/unreachable backend blocks only the push *attempt*, never the
/// checks themselves — `run_cycle` always runs every configured check
/// first, and only then spends time on the network.
pub async fn run(run_config: RunConfig) -> Result<(), AgentError> {
    let token = run_config.resolve_token()?;
    let checks = run_config.load_checks()?;
    let client = IngestClient::new(run_config.backend_url.clone(), run_config.agent_id, token)?;
    let mut buffer = PushBuffer::new(run_config.buffer_capacity);
    let interval = Duration::from_secs(checks.interval_secs.max(1));

    tracing::info!(
        agent = %run_config.name,
        scope = %run_config.scope,
        agent_id = run_config.agent_id,
        checks = checks.checks.len(),
        interval_secs = interval.as_secs(),
        "agent: starting"
    );

    let mut shutdown = std::pin::pin!(shutdown_signal());
    loop {
        run_cycle(&checks.checks, &client, &mut buffer).await;
        tokio::select! {
            () = tokio::time::sleep(interval) => {}
            () = &mut shutdown => {
                tracing::info!("agent: shutdown signal received, attempting one final flush");
                run_cycle(&[], &client, &mut buffer).await;
                break;
            }
        }
    }
    Ok(())
}

/// Runs every configured check, prepends anything still buffered from a
/// previous failed push, and attempts one batched push. On failure the
/// whole batch goes back into the bounded buffer for the next cycle
/// (§7.3) — never blocks waiting for the backend, never panics on a
/// network error.
async fn run_cycle(defs: &[CheckDefinition], client: &IngestClient, buffer: &mut PushBuffer) {
    let mut results = Vec::with_capacity(defs.len());
    for def in defs {
        let outcome = checks::run_check(def).await;
        results.push(PendingResult {
            monitor_id: def.monitor_id(),
            outcome,
        });
    }

    let mut batch = buffer.drain();
    batch.extend(results);

    // Still pushed even when `batch` is empty — a push happening at all is
    // the heartbeat signal (§11.10), independent of whether any check
    // produced something new this cycle.
    match client.push(&batch).await {
        Ok(()) => {
            tracing::debug!(count = batch.len(), "agent: pushed results");
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                count = batch.len(),
                "agent: push failed, buffering for next cycle"
            );
            buffer.push_back_all(batch);
        }
    }
}

/// Resolves on SIGINT, or on SIGTERM where the platform supports it. A
/// failure to install the SIGTERM handler degrades to SIGINT-only rather
/// than panicking the agent (P1 — no `unwrap`/`expect` outside provably-
/// infallible cases; installing a signal handler is a real syscall that can
/// fail under resource exhaustion).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "agent: failed to install SIGTERM handler, only SIGINT will trigger graceful shutdown"
                );
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_token_and_token_file_is_an_error() {
        let config = RunConfig {
            name: "n".to_string(),
            scope: "s".to_string(),
            backend_url: "http://localhost".to_string(),
            agent_id: 1,
            token: None,
            token_file: None,
            config_path: None,
            buffer_capacity: 16,
        };
        assert!(matches!(
            config.resolve_token(),
            Err(AgentError::MissingToken)
        ));
    }

    #[test]
    fn a_plain_token_resolves_directly() {
        let config = RunConfig {
            name: "n".to_string(),
            scope: "s".to_string(),
            backend_url: "http://localhost".to_string(),
            agent_id: 1,
            token: Some("secret".to_string()),
            token_file: None,
            config_path: None,
            buffer_capacity: 16,
        };
        assert_eq!(config.resolve_token().unwrap(), "secret");
    }

    #[test]
    fn a_token_file_is_read_and_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "secret\n").unwrap();
        let config = RunConfig {
            name: "n".to_string(),
            scope: "s".to_string(),
            backend_url: "http://localhost".to_string(),
            agent_id: 1,
            token: None,
            token_file: Some(path),
            config_path: None,
            buffer_capacity: 16,
        };
        assert_eq!(config.resolve_token().unwrap(), "secret");
    }

    #[test]
    fn an_empty_token_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "   \n").unwrap();
        let config = RunConfig {
            name: "n".to_string(),
            scope: "s".to_string(),
            backend_url: "http://localhost".to_string(),
            agent_id: 1,
            token: None,
            token_file: Some(path.clone()),
            config_path: None,
            buffer_capacity: 16,
        };
        assert!(matches!(
            config.resolve_token(),
            Err(AgentError::TokenFileEmpty { path: p }) if p == path
        ));
    }

    #[tokio::test]
    async fn no_config_path_means_zero_checks_not_an_error() {
        let config = RunConfig {
            name: "n".to_string(),
            scope: "s".to_string(),
            backend_url: "http://localhost".to_string(),
            agent_id: 1,
            token: Some("t".to_string()),
            token_file: None,
            config_path: None,
            buffer_capacity: 16,
        };
        let checks = config.load_checks().expect("no config path is valid");
        assert!(checks.checks.is_empty());
    }
}
