//! Shared mutation/query functions (DESIGN.md §4 `backend`, ADR-009).
//!
//! Both the Axum handlers in this crate and `main.rs`'s CLI command
//! execution call these — "a mutation is never implemented twice." Thin
//! wrappers over `Store`; the interesting logic (validation, cascade,
//! `NotFound` mapping) already lives there.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_provider::{ProviderError, Store};

/// `0` on a clock set before the Unix epoch — a misconfigured clock is a
/// distinct failure the engine's own watchdogs will surface (§11.5), not a
/// reason for a registration call to panic (P1: no unwrap()/expect()).
pub(crate) fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub async fn add_monitor(
    store: &dyn Store,
    name: String,
    target: String,
    kind: MonitorKind,
    interval_secs: u64,
    agent_id: Option<u64>,
) -> Result<Monitor, ProviderError> {
    store
        .insert_monitor(Monitor {
            id: 0,
            name,
            target,
            kind,
            interval_secs,
            status: MonitorStatus::Pending,
            agent_id,
        })
        .await
}

pub async fn list_monitors(store: &dyn Store) -> Result<Vec<Monitor>, ProviderError> {
    store.list_monitors().await
}

pub async fn get_monitor(store: &dyn Store, id: u64) -> Result<Option<Monitor>, ProviderError> {
    store.get_monitor(id).await
}

pub async fn edit_monitor(
    store: &dyn Store,
    id: u64,
    name: Option<String>,
    target: Option<String>,
    interval_secs: Option<u64>,
    agent_id: Option<u64>,
) -> Result<(), ProviderError> {
    store
        .update_monitor(id, name, target, interval_secs, agent_id)
        .await
}

pub async fn remove_monitor(store: &dyn Store, id: u64) -> Result<(), ProviderError> {
    store.delete_monitor(id).await
}

pub async fn pause_monitor(store: &dyn Store, id: u64) -> Result<(), ProviderError> {
    store.set_monitor_status(id, MonitorStatus::Paused).await
}

/// Returns to `Pending`, not whatever status the monitor held before it was
/// paused — a resumed monitor hasn't been checked since, so claiming its old
/// status back would be a guess (§5.2, §5.3).
pub async fn resume_monitor(store: &dyn Store, id: u64) -> Result<(), ProviderError> {
    store.set_monitor_status(id, MonitorStatus::Pending).await
}

/// Always issues a fresh token, even for a name that's already registered
/// (§11.10) — that's the mechanism for rotating/revoking an agent's push
/// credential: re-run `agent register` and the old token stops working.
pub async fn register_agent(
    store: &dyn Store,
    name: String,
    scope: String,
) -> Result<Agent, ProviderError> {
    store
        .upsert_agent(Agent {
            id: 0,
            name,
            last_heartbeat_at: now_unix_secs(),
            scope,
            token: monitra_provider::generate_api_token(),
        })
        .await
}

pub async fn list_agents(store: &dyn Store) -> Result<Vec<Agent>, ProviderError> {
    store.list_agents().await
}

pub async fn remove_agent(store: &dyn Store, id: u64) -> Result<(), ProviderError> {
    store.delete_agent(id).await
}

/// A monitor's raw `CheckResult` history, most recent first at the `Store`
/// layer's discretion — read by both `monitra monitor history` and the
/// TUI/web Monitor detail screen's sparkline (Phase 9).
pub async fn monitor_history(
    store: &dyn Store,
    id: u64,
    since: Option<u64>,
) -> Result<Vec<CheckResult>, ProviderError> {
    store.list_check_results(id, since).await
}

/// The global alert feed, across every monitor — read by both
/// `monitra alert list` and the TUI/web Alerts screen (Phase 9).
pub async fn list_all_alert_events(store: &dyn Store) -> Result<Vec<AlertEvent>, ProviderError> {
    store.list_all_alert_events().await
}
