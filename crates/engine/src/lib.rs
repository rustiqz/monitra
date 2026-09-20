//! The monitoring core (DESIGN.md §4 `engine`).
//!
//! Owns the scheduler, probe execution (incl. Collector-based K8s polling,
//! ADR-008), timeout/retry/backoff, concurrency limiting, status-transition
//! logic (flap damping + agent-liveness watchdog), result broadcast, and
//! `AlertEvent` emission (Phase 7). The crate where P1 (reliability)
//! matters most.
//!
//! `EngineHandle::start()` spawns four independent tasks — the scheduler
//! (`scheduler.rs`), the batched writer (`writer.rs`), the agent-liveness
//! watchdog (`watchdog.rs`), and the alert-delivery task (`alerts.rs`,
//! Phase 7) — sharing one `Store` and one broadcast channel of
//! `CheckResult`s (`backend`'s `/ws` fan-out subscribes to it; `backend`
//! never touches the scheduler directly). Agent pushes (`backend`'s
//! `/agents/{id}/ingest`, Phase 7) arrive through [`IngestHandle`] and flow
//! into the same scheduler loop a probe result would.

mod alerts;
mod clock;
mod flap;
pub mod probe;
mod scheduler;
mod watchdog;
mod writer;

use std::sync::Arc;
use std::time::Duration;

use monitra_models::CheckResult;
use monitra_provider::{K8sCollectorFactory, RetryingNotifier, Store};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

pub use alerts::AlertConfig;
pub use scheduler::{PushedResult, SchedulerConfig};
pub use watchdog::WatchdogConfig;

use alerts::Alerts;
use probe::{IcmpProber, Probers};
use scheduler::Scheduler;

/// What `main.rs` hands the engine at startup — the shared `Store`, plus
/// whichever `Collector` factory `collector-kubernetes` built from attached
/// clusters (`None` when the `kubernetes` feature is off or nothing is
/// attached — §4.1: `Collector` has no default), plus the resolved
/// `Notifier` (Phase 7) wrapped in the bounded-queue-with-retry
/// `RetryingNotifier` from `monitra-provider`. `engine` never depends on
/// `collector-kubernetes` or a concrete `Notifier` implementation directly
/// (CLAUDE.md's dependency DAG); only `main.rs` knows which implementations
/// exist.
pub struct EngineDeps {
    pub store: Arc<dyn Store>,
    pub k8s_factory: Option<Arc<dyn K8sCollectorFactory>>,
    pub notifier: Arc<RetryingNotifier>,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub scheduler: SchedulerConfig,
    pub watchdog: WatchdogConfig,
    pub alerts: AlertConfig,
    /// Bound on the writer's inbound channel (§7.3 — every queue has an
    /// explicit bound).
    pub writer_capacity: usize,
    pub writer_batch_size: usize,
    pub writer_flush_interval: Duration,
    /// Bound on the broadcast channel `backend`'s `/ws` fan-out subscribes
    /// to. A slow/absent subscriber never affects this — bounded broadcast
    /// channels drop the oldest message for a lagging receiver, they never
    /// block the sender (§7.2 "WebSocket client stalls").
    pub results_channel_capacity: usize,
    /// Bound on the agent-push ingest channel (§7.3, §6.2 "two producers,
    /// one bounded channel"). A burst of agent pushes drops-and-logs on
    /// overflow rather than back-pressuring `backend`'s ingest handler.
    pub ingest_capacity: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            scheduler: SchedulerConfig::default(),
            watchdog: WatchdogConfig::default(),
            alerts: AlertConfig::default(),
            writer_capacity: 4096,
            writer_batch_size: 200,
            writer_flush_interval: Duration::from_millis(500),
            results_channel_capacity: 1024,
            ingest_capacity: 1024,
        }
    }
}

/// What `backend`'s `POST /agents/{id}/ingest` handler submits pushed
/// results through (Phase 7, ADR-008) — cheap to clone (wraps an
/// `mpsc::Sender`), held in `backend`'s `AppState` for the life of the
/// server.
#[derive(Clone)]
pub struct IngestHandle {
    tx: mpsc::Sender<PushedResult>,
}

impl IngestHandle {
    /// A handle with nothing consuming it. Production code always gets its
    /// `IngestHandle` from [`EngineHandle::ingest_handle`]; this exists for
    /// `backend`'s tests, which need a real `IngestHandle` to construct a
    /// router but usually don't care what happens to submitted results.
    pub fn channel(capacity: usize) -> (Self, mpsc::Receiver<PushedResult>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (Self { tx }, rx)
    }

    /// Never blocks (§7.3, §6.2) — drops and logs loudly on a full queue
    /// rather than back-pressuring the HTTP handler that called this.
    pub fn submit(&self, result: PushedResult) {
        let monitor_id = result.monitor_id;
        if let Err(source) = self.tx.try_send(result) {
            tracing::warn!(
                monitor_id,
                error = %source,
                "engine: ingest queue full, dropping pushed check result"
            );
        }
    }
}

/// A running engine. Dropping this without calling [`EngineHandle::shutdown`]
/// abandons its tasks — always prefer `shutdown` (§7.4).
pub struct EngineHandle {
    shutdown_tx: watch::Sender<bool>,
    scheduler_task: JoinHandle<()>,
    watchdog_task: JoinHandle<()>,
    writer_task: JoinHandle<()>,
    alerts_task: JoinHandle<()>,
    results_tx: broadcast::Sender<CheckResult>,
    ingest: IngestHandle,
}

impl EngineHandle {
    /// Never fails: an unavailable ICMP prober or absent K8s factory
    /// degrade the affected monitors to `Stale`/`Unavailable` rather than
    /// preventing the whole engine from starting (P1).
    pub fn start(deps: EngineDeps, config: EngineConfig) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (results_tx, _unused_receiver) = broadcast::channel(config.results_channel_capacity);
        let (push_tx, push_rx) = mpsc::channel(config.ingest_capacity.max(1));

        let (writer, writer_task) = writer::Writer::spawn(
            Arc::clone(&deps.store),
            config.writer_capacity,
            config.writer_batch_size,
            config.writer_flush_interval,
        );

        let (alerts, alerts_task) =
            Alerts::spawn(Arc::clone(&deps.store), deps.notifier, config.alerts);

        let probers = Arc::new(Probers::new(IcmpProber::new()));
        let scheduler = Scheduler::new(
            Arc::clone(&deps.store),
            probers,
            deps.k8s_factory,
            writer,
            results_tx.clone(),
            alerts.clone(),
            push_rx,
            config.scheduler,
        );
        let scheduler_task = tokio::spawn(scheduler.run(shutdown_rx.clone()));

        let watchdog_task = tokio::spawn(watchdog::run(
            deps.store,
            alerts,
            config.watchdog,
            shutdown_rx,
        ));

        Self {
            shutdown_tx,
            scheduler_task,
            watchdog_task,
            writer_task,
            alerts_task,
            results_tx,
            ingest: IngestHandle { tx: push_tx },
        }
    }

    /// Live `CheckResult`s as they're produced — `backend`'s `/ws` fan-out
    /// subscribes here; the probe/write path never blocks on it (§3.3: "the
    /// probe path and the read path are decoupled").
    pub fn subscribe(&self) -> broadcast::Receiver<CheckResult> {
        self.results_tx.subscribe()
    }

    /// `backend`'s `/ws` handler subscribes its own receiver per connection
    /// (a `broadcast::Receiver` isn't `Clone`), so `backend`'s `AppState`
    /// holds this sender rather than one shared receiver.
    pub fn results_sender(&self) -> broadcast::Sender<CheckResult> {
        self.results_tx.clone()
    }

    /// What `backend`'s agent-ingest handler submits pushed results
    /// through.
    pub fn ingest_handle(&self) -> IngestHandle {
        self.ingest.clone()
    }

    /// §7.4 graceful shutdown: stop scheduling new probes, await in-flight
    /// ones under the scheduler's own deadline, flush the writer, stop the
    /// watchdog and alert-delivery tasks. The writer flushes as a side
    /// effect of the scheduler task ending — it owns the only `Writer` (and
    /// thus the only channel `Sender`), so its drop closes the channel the
    /// writer task is draining. The alert task similarly exits once both
    /// the scheduler's and the watchdog's `Alerts` clones have dropped.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        if let Err(source) = self.scheduler_task.await {
            tracing::warn!(error = %source, "engine: scheduler task panicked during shutdown");
        }
        if let Err(source) = self.watchdog_task.await {
            tracing::warn!(error = %source, "engine: watchdog task panicked during shutdown");
        }
        if let Err(source) = self.writer_task.await {
            tracing::warn!(error = %source, "engine: writer task panicked during shutdown");
        }
        if let Err(source) = self.alerts_task.await {
            tracing::warn!(error = %source, "engine: alerts task panicked during shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §7.3/§6.2 gate: a burst of agent pushes that outruns the scheduler
    /// drops-and-logs on overflow rather than blocking the ingest handler.
    /// Constructed directly against a capacity-1 channel with nothing
    /// draining it, so overflow is deterministic rather than racing the
    /// real scheduler loop.
    #[test]
    fn ingest_submit_drops_and_logs_without_blocking_when_queue_is_full() {
        let (tx, mut rx) = mpsc::channel(1);
        let ingest = IngestHandle { tx };

        ingest.submit(PushedResult {
            monitor_id: 1,
            success: true,
            latency_ms: 5,
            message: None,
        });
        ingest.submit(PushedResult {
            monitor_id: 2,
            success: false,
            latency_ms: 0,
            message: Some("dropped".to_string()),
        });

        let received = rx.try_recv().expect("first push was queued");
        assert_eq!(received.monitor_id, 1);
        assert!(
            rx.try_recv().is_err(),
            "second push must have been dropped, not queued behind the first"
        );
    }
}
