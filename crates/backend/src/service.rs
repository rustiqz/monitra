//! Shared mutation/query functions (DESIGN.md §4 `backend`, ADR-009).
//!
//! Both the Axum handlers in this crate and `main.rs`'s CLI command
//! execution call these — "a mutation is never implemented twice." Thin
//! wrappers over `Store`; the interesting logic (validation, cascade,
//! `NotFound` mapping) already lives there.

use std::collections::HashMap;
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
/// `region` is fully overwritten too (ADR-011) — omitting it on a repeat
/// registration clears any previously stored region, the same rule already
/// applied to `scope`/`token`.
pub async fn register_agent(
    store: &dyn Store,
    name: String,
    scope: String,
    region: Option<String>,
) -> Result<Agent, ProviderError> {
    store
        .upsert_agent(Agent {
            id: 0,
            name,
            last_heartbeat_at: now_unix_secs(),
            scope,
            token: monitra_provider::generate_api_token(),
            region,
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

/// Per-region latency/failure-rate comparison for one target (ADR-011,
/// Phase 11) — not a persisted entity (§5.1's "resist adding entities"
/// discipline holds), composed read-side from `Monitor`/`Agent`/
/// `CheckResult` every time this is called. Read by both `monitra monitor
/// regions` and `GET /regions`.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionAggregate {
    pub target: String,
    pub kind: MonitorKind,
    pub region: String,
    /// How many distinct `Monitor` rows contributed to this group — usually
    /// one (one monitor per region per target), but nothing enforces that.
    pub monitor_count: usize,
    pub probe_count: usize,
    pub failure_count: usize,
    /// Nearest-rank p95 over successful probes' `latency_ms` only — a
    /// failed probe's latency is `0` (§4 `engine`'s `apply_result`), which
    /// would understate latency if it were mixed in. `0` when there have
    /// been no successful probes at all, not a fabricated number (P1).
    pub p95_latency_ms: u64,
}

/// Groups every network-probe `Monitor` that is both agent-linked and whose
/// agent has a declared region, by `(target, kind, region)`, then pulls
/// each group's `CheckResult`s (optionally bounded by `since`, same
/// semantics as [`monitor_history`]) to compute per-region aggregates.
///
/// A monitor whose `agent_id` is `None`, or whose linked agent has no
/// `region`, contributes to no aggregate at all — ADR-011's "an agent with
/// no declared region is excluded from regional aggregation, never
/// guessed" applies at this composition step, not just at storage.
pub async fn region_aggregates(
    store: &dyn Store,
    since: Option<u64>,
) -> Result<Vec<RegionAggregate>, ProviderError> {
    let monitors = store.list_monitors().await?;
    let agents = store.list_agents().await?;
    let region_by_agent_id: HashMap<u64, String> = agents
        .into_iter()
        .filter_map(|agent| agent.region.map(|region| (agent.id, region)))
        .collect();

    let mut groups: HashMap<(String, MonitorKind, String), (usize, Vec<CheckResult>)> =
        HashMap::new();
    for monitor in monitors {
        if !matches!(
            monitor.kind,
            MonitorKind::Http | MonitorKind::Tcp | MonitorKind::Icmp
        ) {
            continue;
        }
        let Some(agent_id) = monitor.agent_id else {
            continue;
        };
        let Some(region) = region_by_agent_id.get(&agent_id) else {
            continue;
        };
        let results = store.list_check_results(monitor.id, since).await?;
        let entry = groups
            .entry((monitor.target, monitor.kind, region.clone()))
            .or_insert_with(|| (0, Vec::new()));
        entry.0 += 1;
        entry.1.extend(results);
    }

    let mut aggregates: Vec<RegionAggregate> = groups
        .into_iter()
        .map(|((target, kind, region), (monitor_count, results))| {
            let probe_count = results.len();
            let failure_count = results.iter().filter(|r| !r.success).count();
            RegionAggregate {
                target,
                kind,
                region,
                monitor_count,
                probe_count,
                failure_count,
                p95_latency_ms: p95_latency_ms(&results),
            }
        })
        .collect();
    aggregates.sort_by(|a, b| (&a.target, &a.region).cmp(&(&b.target, &b.region)));
    Ok(aggregates)
}

/// Nearest-rank p95 over successful results' `latency_ms` only. `0` for no
/// successful results — an honest "nothing to report," not a guess.
fn p95_latency_ms(results: &[CheckResult]) -> u64 {
    let mut latencies: Vec<u64> = results
        .iter()
        .filter(|r| r.success)
        .map(|r| r.latency_ms)
        .collect();
    if latencies.is_empty() {
        return 0;
    }
    latencies.sort_unstable();
    let rank = ((latencies.len() as f64) * 0.95).ceil() as usize;
    let index = rank.saturating_sub(1).min(latencies.len() - 1);
    latencies[index]
}
