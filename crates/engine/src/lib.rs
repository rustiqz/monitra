//! The monitoring core (DESIGN.md §4 `engine`).
//!
//! Owns the scheduler, probe execution (incl. Collector-based K8s polling,
//! ADR-008), timeout/retry/backoff, concurrency limiting, status-transition
//! logic (flap damping + agent-liveness watchdog), result broadcast, and
//! `AlertEvent` emission (Phase 7). The crate where P1 (reliability)
//! matters most.
//!
//! `EngineHandle::start()` spawns three independent tasks — the scheduler
//! (`scheduler.rs`), the batched writer (`writer.rs`), and the agent-
//! liveness watchdog (`watchdog.rs`) — sharing one `Store` and one
//! broadcast channel of `CheckResult`s (Phase 7's WebSocket fan-out
//! subscribes to it; `backend` never touches the scheduler directly).

mod clock;
mod flap;
pub mod probe;
mod scheduler;
mod watchdog;
mod writer;

use std::sync::Arc;
use std::time::Duration;

use monitra_models::CheckResult;
use monitra_provider::{K8sCollectorFactory, Store};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

pub use scheduler::SchedulerConfig;
pub use watchdog::WatchdogConfig;

use probe::{IcmpProber, Probers};
use scheduler::Scheduler;

/// What `main.rs` hands the engine at startup — the shared `Store`, plus
/// whichever `Collector` factory `collector-kubernetes` built from attached
/// clusters (`None` when the `kubernetes` feature is off or nothing is
/// attached — §4.1: `Collector` has no default). `engine` never depends on
/// `collector-kubernetes` directly (CLAUDE.md's dependency DAG); only
/// `main.rs` knows which implementations exist.
pub struct EngineDeps {
    pub store: Arc<dyn Store>,
    pub k8s_factory: Option<Arc<dyn K8sCollectorFactory>>,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub scheduler: SchedulerConfig,
    pub watchdog: WatchdogConfig,
    /// Bound on the writer's inbound channel (§7.3 — every queue has an
    /// explicit bound).
    pub writer_capacity: usize,
    pub writer_batch_size: usize,
    pub writer_flush_interval: Duration,
    /// Bound on the broadcast channel Phase 7's WebSocket fan-out will
    /// subscribe to. A slow/absent subscriber never affects this — bounded
    /// broadcast channels drop the oldest message for a lagging receiver,
    /// they never block the sender (§7.2 "WebSocket client stalls").
    pub results_channel_capacity: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            scheduler: SchedulerConfig::default(),
            watchdog: WatchdogConfig::default(),
            writer_capacity: 4096,
            writer_batch_size: 200,
            writer_flush_interval: Duration::from_millis(500),
            results_channel_capacity: 1024,
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
    results_tx: broadcast::Sender<CheckResult>,
}

impl EngineHandle {
    /// Never fails: an unavailable ICMP prober or absent K8s factory
    /// degrade the affected monitors to `Stale`/`Unavailable` rather than
    /// preventing the whole engine from starting (P1).
    pub fn start(deps: EngineDeps, config: EngineConfig) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (results_tx, _unused_receiver) = broadcast::channel(config.results_channel_capacity);

        let (writer, writer_task) = writer::Writer::spawn(
            Arc::clone(&deps.store),
            config.writer_capacity,
            config.writer_batch_size,
            config.writer_flush_interval,
        );

        let probers = Arc::new(Probers::new(IcmpProber::new()));
        let scheduler = Scheduler::new(
            Arc::clone(&deps.store),
            probers,
            deps.k8s_factory,
            writer,
            results_tx.clone(),
            config.scheduler,
        );
        let scheduler_task = tokio::spawn(scheduler.run(shutdown_rx.clone()));

        let watchdog_task = tokio::spawn(watchdog::run(deps.store, config.watchdog, shutdown_rx));

        Self {
            shutdown_tx,
            scheduler_task,
            watchdog_task,
            writer_task,
            results_tx,
        }
    }

    /// Live `CheckResult`s as they're produced — Phase 7's WebSocket
    /// fan-out subscribes here; the probe/write path never blocks on it
    /// (§3.3: "the probe path and the read path are decoupled").
    pub fn subscribe(&self) -> broadcast::Receiver<CheckResult> {
        self.results_tx.subscribe()
    }

    /// §7.4 graceful shutdown: stop scheduling new probes, await in-flight
    /// ones under the scheduler's own deadline, flush the writer, stop the
    /// watchdog. The writer flushes as a side effect of the scheduler task
    /// ending — it owns the only `Writer` (and thus the only channel
    /// `Sender`), so its drop closes the channel the writer task is
    /// draining.
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
    }
}
