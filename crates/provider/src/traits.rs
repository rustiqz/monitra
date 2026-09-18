//! The `Store`, `Cache`, `Notifier`, and `Collector` contracts (DESIGN.md §4
//! `provider`, ADR-007, ADR-008).
//!
//! Deliberately minimal as of Phase 3: just enough surface for the §4.1
//! availability policies to be implemented and tested against fakes. Each
//! trait grows the CRUD/domain-shaped methods it actually needs when the
//! phase that needs them arrives (`storage` at Phase 4, notifier sinks at
//! Phase 7, `collector-kubernetes` polling at Phase 6) — adding methods to a
//! trait is not a breaking change to callers that only hold `Arc<dyn Trait>`
//! through the subset they already use.
//!
//! Traits are `async` (via `async_trait`, for dyn-safety) because `engine`
//! and `backend` hold these behind `Arc<dyn Store>` etc. (CLAUDE.md) — a
//! sync trait now would mean redesigning every implementor once real I/O
//! lands in Phase 4+.

use async_trait::async_trait;

use crate::error::ProviderError;
use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorStatus};

/// A `Store` provider (default: SQLite, `storage`; alternative: Postgres,
/// `store-postgres`). Unreachable-when-configured fails the daemon fast
/// (§4.1) — there is no degrade path for this trait.
///
/// CRUD surface added at Phase 4, scoped to what `storage`'s SQLite impl and
/// its tests need now. `engine`'s batched writer task (§6.2, Phase 6) is the
/// only caller expected to use `insert_check_results` with more than one
/// result at a time — `storage` itself does no batching or scheduling.
#[async_trait]
pub trait Store: Send + Sync {
    /// Stable name for logging/`/health` (e.g. `"sqlite"`, `"postgres"`).
    fn name(&self) -> &'static str;

    /// Cheap reachability check, used at startup to decide fail-fast.
    async fn health_check(&self) -> Result<(), ProviderError>;

    /// Registers a new monitor, returning it with its assigned `id`.
    async fn insert_monitor(&self, monitor: Monitor) -> Result<Monitor, ProviderError>;

    async fn get_monitor(&self, id: u64) -> Result<Option<Monitor>, ProviderError>;

    async fn list_monitors(&self) -> Result<Vec<Monitor>, ProviderError>;

    /// Updates whichever of `name`/`target`/`interval_secs` is `Some`,
    /// leaving the rest unchanged.
    async fn update_monitor(
        &self,
        id: u64,
        name: Option<String>,
        target: Option<String>,
        interval_secs: Option<u64>,
    ) -> Result<(), ProviderError>;

    async fn set_monitor_status(&self, id: u64, status: MonitorStatus)
    -> Result<(), ProviderError>;

    /// Removes a monitor and cascades to its `check_results`/`alert_events`
    /// — nothing reads history for a monitor that no longer exists.
    async fn delete_monitor(&self, id: u64) -> Result<(), ProviderError>;

    /// Batched insert — the shape `engine`'s writer task (§6.2) needs, one
    /// transaction regardless of batch size.
    async fn insert_check_results(&self, results: &[CheckResult]) -> Result<(), ProviderError>;

    async fn list_check_results(
        &self,
        monitor_id: u64,
        since: Option<u64>,
    ) -> Result<Vec<CheckResult>, ProviderError>;

    /// Deletes raw `check_results` older than `cutoff_unix_secs` (§5.4).
    /// Returns the number of rows removed.
    async fn prune_check_results_older_than(
        &self,
        cutoff_unix_secs: u64,
    ) -> Result<u64, ProviderError>;

    /// Registers or updates an agent by name, returning it with its `id`.
    async fn upsert_agent(&self, agent: Agent) -> Result<Agent, ProviderError>;

    async fn heartbeat_agent(&self, id: u64, at_unix_secs: u64) -> Result<(), ProviderError>;

    async fn get_agent(&self, id: u64) -> Result<Option<Agent>, ProviderError>;

    async fn list_agents(&self) -> Result<Vec<Agent>, ProviderError>;

    /// Persists an emitted alert, returning it with its assigned `id`.
    async fn insert_alert_event(&self, event: AlertEvent) -> Result<AlertEvent, ProviderError>;

    async fn list_alert_events(&self, monitor_id: u64) -> Result<Vec<AlertEvent>, ProviderError>;
}

/// A `Cache` provider (default: `InProcessCache`; alternative: Redis,
/// `cache-redis`). Unreachable-when-configured degrades to the in-process
/// default and logs at WARN (§4.1) — a cache miss costs latency, nothing more.
#[async_trait]
pub trait Cache: Send + Sync {
    fn name(&self) -> &'static str;

    async fn get(&self, key: &str) -> Result<Option<String>, ProviderError>;

    async fn set(&self, key: &str, value: &str) -> Result<(), ProviderError>;
}

/// A `Notifier` provider (default: webhook, `notify-webhook`, logging when no
/// target is configured; alternatives: `notify-slack`, others). Unreachable-
/// when-configured is queued with bounded retry and backoff, never blocks a
/// probe (§4.1).
#[async_trait]
pub trait Notifier: Send + Sync {
    fn name(&self) -> &'static str;

    async fn notify(&self, message: &str) -> Result<(), ProviderError>;
}

/// Result of polling one `Collector`-backed resource. Never propagated as a
/// hard error to the caller — an unreachable cluster marks the affected
/// resource `Unknown`, on the same honesty grounds as `Pending` (§5.2), and
/// never fails the daemon (§4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectorStatus {
    Healthy,
    Unhealthy { reason: String },
    Unknown { reason: String },
}

/// A `Collector` provider (e.g. `collector-kubernetes`, ADR-008). Has no
/// embedded default — it only exists because an operator configured one
/// (§4.1).
#[async_trait]
pub trait Collector: Send + Sync {
    fn name(&self) -> &'static str;

    /// Poll the one resource this collector instance watches. Errors are a
    /// collector-internal detail; callers should prefer `poll_collector_safely`
    /// (`policy.rs`) over calling this directly, since it converts errors into
    /// `CollectorStatus::Unknown` instead of letting them propagate.
    async fn poll(&self) -> Result<CollectorStatus, ProviderError>;
}
