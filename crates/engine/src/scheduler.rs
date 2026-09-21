//! The scheduler (DESIGN.md §6.2, §6.3, §3.3): task-per-monitor dispatch,
//! a semaphore bounding in-flight probes (not monitor count), flap-damped
//! status transitions, and periodic re-sync with storage so a running
//! daemon picks up monitors added/edited/removed by the one-shot CLI
//! (§3.4 — `monitor add` etc. work with or without a daemon running, which
//! means storage is the source of truth this loop must keep polling, not
//! something it's ever told about directly).
//!
//! All status-transition and write logic runs back in this single loop
//! (via `Outcome`, returned by each spawned task) rather than inside the
//! spawned probe tasks themselves — that keeps `FlapState` plain, owned
//! data instead of `Arc<Mutex<_>>` per monitor.
//!
//! A network-probe monitor whose `agent_id` names a region-tagged agent
//! (ADR-011, Phase 11) is never dispatched here at all — due-computation
//! stays central, but the actual probe runs on that agent's own vantage
//! point. `dispatch_due` hands it to [`AssignmentHandle`] instead of
//! spawning a task; the agent pulls it via `backend`'s
//! `GET /agents/{id}/assignments` and pushes the result back through the
//! same ingest path a `HostAgentCheck` result already uses.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant as StdInstant};

use monitra_models::{CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_probe::{NetworkProbeKind, ProbeOutcome, Probers};
use monitra_provider::{Collector, CollectorStatus, K8sCollectorFactory, Store};
use tokio::sync::{Semaphore, broadcast, mpsc, watch};
use tokio::task::{Id as TaskId, JoinError, JoinSet};
use tokio::time::Instant;

use crate::alerts::{AlertRequest, Alerts};
use crate::clock::now_unix_secs;
use crate::flap::FlapState;
use crate::writer::Writer;

/// One network-probe monitor assigned to a region-tagged agent (ADR-011).
/// The engine still owns due-computation (§6.3) — this is handed off only
/// once a check is actually due, not the monitor's full config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub monitor_id: u64,
    pub target: String,
    pub kind: MonitorKind,
}

/// Per-agent bounded queues of due regional probes (ADR-011, Phase 11) —
/// mirrors [`crate::IngestHandle`]'s shape (cheap-clone, `Arc`-backed,
/// never blocks) but inverted: the scheduler is the producer here, and
/// `backend`'s `GET /agents/{id}/assignments` handler is the consumer,
/// pulling on the agent's own cadence rather than having results pushed to
/// it. Each agent's queue is independent, so one agent falling behind never
/// affects another's.
#[derive(Clone)]
pub struct AssignmentHandle {
    capacity: usize,
    queues: Arc<Mutex<HashMap<u64, VecDeque<Assignment>>>>,
}

impl AssignmentHandle {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            queues: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Called by the scheduler when a network-probe monitor linked to
    /// `agent_id` becomes due. Drops the oldest queued assignment for that
    /// agent and logs loudly on overflow (§7.3) rather than growing without
    /// bound while an agent is slow, offline, or simply not polling yet.
    /// `pub` (not just scheduler-internal) so `backend`'s tests can seed a
    /// handle directly, the same way `IngestHandle::channel` lets `backend`
    /// tests construct a router without a real scheduler behind it.
    pub fn enqueue(&self, agent_id: u64, assignment: Assignment) {
        let mut queues = self.queues.lock().unwrap_or_else(PoisonError::into_inner);
        let queue = queues.entry(agent_id).or_default();
        if queue.len() >= self.capacity {
            let dropped = queue.pop_front();
            tracing::warn!(
                agent_id,
                dropped_monitor_id = dropped.map(|a| a.monitor_id),
                capacity = self.capacity,
                "engine: assignment queue full for agent, dropping oldest pending regional probe"
            );
        }
        queue.push_back(assignment);
    }

    /// Drains everything currently queued for `agent_id` — what `backend`'s
    /// `GET /agents/{id}/assignments` handler returns. Empty is the common
    /// case (nothing due right now), not an error.
    pub fn drain(&self, agent_id: u64) -> Vec<Assignment> {
        let mut queues = self.queues.lock().unwrap_or_else(PoisonError::into_inner);
        queues
            .get_mut(&agent_id)
            .map(|queue| queue.drain(..).collect())
            .unwrap_or_default()
    }
}

/// One result an agent pushed via `POST /agents/{id}/ingest` (§4 `backend`,
/// ADR-008), handed to the scheduler through a bounded channel rather than
/// a spawned task — there is no probe to run, the result already exists.
/// Carries no agent identity: `backend` has already checked the pushed
/// `monitor_id` belongs to the authenticated agent before this is
/// submitted, so once it arrives here it is indistinguishable from a
/// scheduler-dispatched probe's result (§4 `engine`, §6.2).
///
/// `outcome` reuses [`ProbeOutcome`] rather than a flat `success: bool`
/// (Phase 8) — a `HostAgentCheck` that failed to run at all (permission
/// denied, `systemctl`/dbus unreachable) is an `Unavailable`, not a
/// `Failure`: the same "our side, not the target" honesty §11.3/P1 already
/// require of the pull path.
pub struct PushedResult {
    pub monitor_id: u64,
    pub outcome: ProbeOutcome,
}

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub max_concurrent_probes: usize,
    pub probe_timeout: Duration,
    pub resync_interval: Duration,
    /// §7.4 step 2: deadline for in-flight probes to finish during shutdown.
    pub shutdown_deadline: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_probes: 64,
            probe_timeout: Duration::from_secs(10),
            resync_interval: Duration::from_secs(5),
            shutdown_deadline: Duration::from_secs(10),
        }
    }
}

struct MonitorState {
    monitor: Monitor,
    next_check_at: Instant,
    flap: FlapState,
}

/// What a spawned task hands back to the scheduler loop.
enum Outcome {
    Network {
        monitor_id: u64,
        outcome: ProbeOutcome,
    },
    Collector {
        monitor_id: u64,
        status: CollectorStatus,
        latency_ms: u64,
    },
}

/// Which monitor a spawned task belongs to, keyed by `tokio::task::Id` —
/// tracked outside `JoinSet` so a task that panics (rather than returning
/// an `Outcome`) can still be attributed to a monitor (§7.2, §11.6). Kept
/// as `&mut` parameters alongside `tasks: &mut JoinSet<Outcome>` rather
/// than on `Scheduler` itself, matching how `tasks` is already threaded
/// through `dispatch_due`/`dispatch_network`/`dispatch_k8s`.
#[derive(Clone, Copy)]
enum TaskOwner {
    Network(u64),
    Collector(u64),
}

/// Extracts a human-readable reason from a `JoinError` — either the
/// panic payload (when it's a `&str`/`String`, which covers `panic!`,
/// `unwrap`/`expect`, and `unreachable!`) or a fixed message for the
/// cancellation case (never triggered today; nothing calls `abort()`).
fn join_error_message(err: JoinError) -> String {
    if err.is_cancelled() {
        return "task was cancelled before completing".to_string();
    }
    match err.try_into_panic() {
        Ok(payload) => payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic payload was not a string".to_string()),
        Err(_) => "task ended abnormally".to_string(),
    }
}

pub struct Scheduler {
    store: Arc<dyn Store>,
    probers: Arc<Probers>,
    k8s_factory: Option<Arc<dyn K8sCollectorFactory>>,
    semaphore: Arc<Semaphore>,
    writer: Writer,
    results_tx: broadcast::Sender<CheckResult>,
    alerts: Alerts,
    push_rx: mpsc::Receiver<PushedResult>,
    assignments: AssignmentHandle,
    config: SchedulerConfig,
    registry: HashMap<u64, MonitorState>,
    k8s_collectors: HashMap<u64, Arc<dyn Collector>>,
}

impl Scheduler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<dyn Store>,
        probers: Arc<Probers>,
        k8s_factory: Option<Arc<dyn K8sCollectorFactory>>,
        writer: Writer,
        results_tx: broadcast::Sender<CheckResult>,
        alerts: Alerts,
        push_rx: mpsc::Receiver<PushedResult>,
        assignments: AssignmentHandle,
        config: SchedulerConfig,
    ) -> Self {
        Self {
            store,
            probers,
            k8s_factory,
            semaphore: Arc::new(Semaphore::new(config.max_concurrent_probes.max(1))),
            writer,
            results_tx,
            alerts,
            push_rx,
            assignments,
            config,
            registry: HashMap::new(),
            k8s_collectors: HashMap::new(),
        }
    }

    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        self.resync().await;
        let mut resync_ticker = tokio::time::interval(self.config.resync_interval);
        resync_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut tasks: JoinSet<Outcome> = JoinSet::new();
        let mut owners: HashMap<TaskId, TaskOwner> = HashMap::new();

        loop {
            let deadline = self.next_deadline();
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                _ = resync_ticker.tick() => {
                    self.resync().await;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    self.dispatch_due(&mut tasks, &mut owners);
                }
                Some(joined) = tasks.join_next_with_id(), if !tasks.is_empty() => {
                    self.handle_joined(joined, &mut owners).await;
                }
                received = self.push_rx.recv() => {
                    if let Some(pushed) = received {
                        self.handle_pushed(pushed).await;
                    }
                }
            }
        }

        tracing::info!("engine: scheduler stopping, draining in-flight probes");
        let deadline = self.config.shutdown_deadline;
        let drain = async {
            while let Some(joined) = tasks.join_next_with_id().await {
                self.handle_joined(joined, &mut owners).await;
            }
        };
        if tokio::time::timeout(deadline, drain).await.is_err() {
            tracing::warn!(
                abandoned = tasks.len(),
                "engine: shutdown deadline exceeded, abandoning remaining in-flight probes"
            );
        }
    }

    fn next_deadline(&self) -> Instant {
        self.registry
            .values()
            .map(|state| state.next_check_at)
            .min()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600))
    }

    /// Reconciles the in-memory registry against storage — picks up
    /// monitors added/removed/edited since the last sync, and status
    /// changes made outside this loop (a resumed-from-pause monitor, or the
    /// watchdog demoting one to `Stale`). Preserves `next_check_at`/`FlapState`
    /// for monitors that already existed so a resync never resets damping
    /// progress or causes a burst of simultaneous re-checks.
    async fn resync(&mut self) {
        let monitors = match self.store.list_monitors().await {
            Ok(monitors) => monitors,
            Err(source) => {
                tracing::warn!(error = %source, "engine: resync failed to list monitors, keeping previous registry");
                return;
            }
        };

        let now = Instant::now();
        let seen: HashSet<u64> = monitors
            .iter()
            .filter(|m| m.status != MonitorStatus::Paused)
            .map(|m| m.id)
            .collect();
        self.registry.retain(|id, _| seen.contains(id));
        self.k8s_collectors.retain(|id, _| seen.contains(id));

        for monitor in monitors {
            if monitor.status == MonitorStatus::Paused {
                continue;
            }
            match self.registry.get_mut(&monitor.id) {
                Some(state) => state.monitor = monitor,
                None => {
                    self.registry.insert(
                        monitor.id,
                        MonitorState {
                            monitor,
                            next_check_at: now,
                            flap: FlapState::default(),
                        },
                    );
                }
            }
        }
    }

    fn dispatch_due(
        &mut self,
        tasks: &mut JoinSet<Outcome>,
        owners: &mut HashMap<TaskId, TaskOwner>,
    ) {
        let now = Instant::now();
        let due: Vec<u64> = self
            .registry
            .iter()
            .filter(|(_, state)| state.next_check_at <= now)
            .map(|(id, _)| *id)
            .collect();

        for monitor_id in due {
            let Some(state) = self.registry.get_mut(&monitor_id) else {
                continue;
            };
            let interval = Duration::from_secs(state.monitor.interval_secs.max(1));
            let previous_deadline = state.next_check_at;
            state.next_check_at = advance_deadline(previous_deadline, interval, now);
            if state.next_check_at != previous_deadline + interval {
                tracing::warn!(
                    monitor_id,
                    behind_by_secs = (now - previous_deadline).as_secs(),
                    "engine: scheduler fell behind on this monitor, resetting its deadline instead of bursting catch-up checks"
                );
            }
            let monitor = state.monitor.clone();

            match NetworkProbeKind::try_from(monitor.kind) {
                Ok(kind) => match monitor.agent_id {
                    // Region-tagged agent (ADR-011, Phase 11): the engine
                    // still decides the check is due, but hands it off to
                    // that agent's pull queue instead of probing it
                    // centrally — the whole point is a *different* vantage
                    // point than this process's own network path.
                    Some(agent_id) => self.assignments.enqueue(
                        agent_id,
                        Assignment {
                            monitor_id,
                            target: monitor.target.clone(),
                            kind: monitor.kind,
                        },
                    ),
                    None => self.dispatch_network(monitor_id, kind, &monitor.target, tasks, owners),
                },
                Err(()) if monitor.kind == MonitorKind::HostAgentCheck => {
                    // Pushed by agents (Phase 8) — never scheduler-dispatched.
                }
                Err(()) => self.dispatch_k8s(monitor_id, &monitor, tasks, owners),
            }
        }
    }

    fn dispatch_network(
        &self,
        monitor_id: u64,
        kind: NetworkProbeKind,
        target: &str,
        tasks: &mut JoinSet<Outcome>,
        owners: &mut HashMap<TaskId, TaskOwner>,
    ) {
        let probers = Arc::clone(&self.probers);
        let semaphore = Arc::clone(&self.semaphore);
        let timeout = self.config.probe_timeout;
        let target = target.to_string();
        let handle = tasks.spawn(async move {
            // Acquiring the permit here, inside the task, means dispatch
            // itself never blocks waiting for a free slot — only the probe
            // does (§6.2: the semaphore bounds in-flight probes, not the
            // scheduler's own clock). Never closed, so never actually Err.
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("engine: probe semaphore is never closed");
            let outcome = probers.probe(kind, &target, timeout).await;
            Outcome::Network {
                monitor_id,
                outcome,
            }
        });
        owners.insert(handle.id(), TaskOwner::Network(monitor_id));
    }

    fn dispatch_k8s(
        &mut self,
        monitor_id: u64,
        monitor: &Monitor,
        tasks: &mut JoinSet<Outcome>,
        owners: &mut HashMap<TaskId, TaskOwner>,
    ) {
        let collector = self.k8s_collectors.get(&monitor_id).cloned().or_else(|| {
            let (factory, target) = (self.k8s_factory.as_ref()?, parse_k8s_target(&monitor.target)?);
            let (cluster, namespace, name) = target;
            match factory.collector_for(cluster, namespace, name, monitor.kind) {
                Ok(collector) => {
                    self.k8s_collectors.insert(monitor_id, Arc::clone(&collector));
                    Some(collector)
                }
                Err(source) => {
                    tracing::warn!(monitor_id, error = %source, "engine: failed to build k8s collector");
                    None
                }
            }
        });

        let Some(collector) = collector else {
            if self.k8s_factory.is_none() {
                tracing::warn!(
                    monitor_id,
                    "engine: k8s monitor configured but no Kubernetes collector is available \
                     (kubernetes feature disabled or no clusters attached)"
                );
            } else if parse_k8s_target(&monitor.target).is_none() {
                tracing::warn!(
                    monitor_id,
                    target = %monitor.target,
                    "engine: k8s monitor target is not in '<cluster>/<namespace>/<name>' form"
                );
            }
            let handle = tasks.spawn(async move {
                Outcome::Collector {
                    monitor_id,
                    status: CollectorStatus::Unknown {
                        reason: "collector unavailable".to_string(),
                    },
                    latency_ms: 0,
                }
            });
            owners.insert(handle.id(), TaskOwner::Collector(monitor_id));
            return;
        };

        let semaphore = Arc::clone(&self.semaphore);
        let handle = tasks.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("engine: probe semaphore is never closed");
            let started = StdInstant::now();
            let status = monitra_provider::poll_collector_safely(collector.as_ref()).await;
            let latency_ms = started.elapsed().as_millis() as u64;
            Outcome::Collector {
                monitor_id,
                status,
                latency_ms,
            }
        });
        owners.insert(handle.id(), TaskOwner::Collector(monitor_id));
    }

    /// Routes one `join_next_with_id` result. On `Ok`, this is just
    /// bookkeeping cleanup before handing off to `handle_outcome`. On
    /// `Err` — a probe or collector task panicked (or, theoretically, was
    /// cancelled, though nothing calls `abort()` today) — the task never
    /// produced an `Outcome`, so previously this branch silently dropped
    /// the check entirely (no log, no recorded status): a silent drop that
    /// violated P1/§7.3 and contradicted §7.2's own "caught via JoinHandle
    /// error; recorded as an internal error" promise. Resolved at Phase 12
    /// (§11.6) by looking the task back up in `owners` and routing a
    /// synthesized `Unavailable`/`Unknown` through the normal
    /// `handle_outcome` path — same "our side, not target-down" treatment
    /// collector-unavailable already gets.
    async fn handle_joined(
        &mut self,
        joined: Result<(TaskId, Outcome), JoinError>,
        owners: &mut HashMap<TaskId, TaskOwner>,
    ) {
        match joined {
            Ok((id, outcome)) => {
                owners.remove(&id);
                self.handle_outcome(outcome).await;
            }
            Err(err) => {
                let id = err.id();
                let owner = owners.remove(&id);
                let message = join_error_message(err);
                match owner {
                    Some(TaskOwner::Network(monitor_id)) => {
                        tracing::error!(
                            monitor_id,
                            task_id = %id,
                            reason = %message,
                            "engine: probe task did not complete; recording as unavailable instead of dropping the check"
                        );
                        self.handle_outcome(Outcome::Network {
                            monitor_id,
                            outcome: ProbeOutcome::Unavailable { message },
                        })
                        .await;
                    }
                    Some(TaskOwner::Collector(monitor_id)) => {
                        tracing::error!(
                            monitor_id,
                            task_id = %id,
                            reason = %message,
                            "engine: collector task did not complete; recording as unknown instead of dropping the check"
                        );
                        self.handle_outcome(Outcome::Collector {
                            monitor_id,
                            status: CollectorStatus::Unknown { reason: message },
                            latency_ms: 0,
                        })
                        .await;
                    }
                    None => {
                        tracing::error!(
                            task_id = %id,
                            reason = %message,
                            "engine: scheduler task did not complete and has no recorded owner; no monitor to attribute this to"
                        );
                    }
                }
            }
        }
    }

    async fn handle_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Network {
                monitor_id,
                outcome,
            } => {
                let (success, message, latency_ms) = match outcome {
                    ProbeOutcome::Success { latency_ms } => (true, None, latency_ms),
                    ProbeOutcome::Failure { message } => (false, Some(message), 0),
                    ProbeOutcome::Unavailable { message } => {
                        self.mark_stale_and_record(monitor_id, message).await;
                        return;
                    }
                };
                self.apply_result(monitor_id, success, message, latency_ms)
                    .await;
            }
            Outcome::Collector {
                monitor_id,
                status,
                latency_ms,
            } => {
                let (success, message) = match status {
                    CollectorStatus::Healthy => (true, None),
                    CollectorStatus::Unhealthy { reason } => (false, Some(reason)),
                    CollectorStatus::Unknown { reason } => {
                        self.mark_stale_and_record(monitor_id, reason).await;
                        return;
                    }
                };
                self.apply_result(monitor_id, success, message, latency_ms)
                    .await;
            }
        }
    }

    /// A `HostAgentCheck` result pushed by an agent (§4 `agent`, ADR-008).
    /// Same three-way split as `Outcome::Network` — `Unavailable` bypasses
    /// flap damping into `Stale` rather than counting as a failed check.
    async fn handle_pushed(&mut self, pushed: PushedResult) {
        let (success, message, latency_ms) = match pushed.outcome {
            ProbeOutcome::Success { latency_ms } => (true, None, latency_ms),
            ProbeOutcome::Failure { message } => (false, Some(message), 0),
            ProbeOutcome::Unavailable { message } => {
                self.mark_stale_and_record(pushed.monitor_id, message).await;
                return;
            }
        };
        self.apply_result(pushed.monitor_id, success, message, latency_ms)
            .await;
    }

    /// §5.1: an unreachable probe/collector on *our* side is `Stale`, never
    /// flap-damped toward `Down` — damping is for "is the target actually
    /// failing", not "can we even tell right now."
    async fn mark_stale_and_record(&mut self, monitor_id: u64, message: String) {
        let checked_at = now_unix_secs();
        if let Some(state) = self.registry.get_mut(&monitor_id)
            && state.monitor.status != MonitorStatus::Stale
        {
            state.monitor.status = MonitorStatus::Stale;
            let monitor_name = state.monitor.name.clone();
            self.persist_status(monitor_id, MonitorStatus::Stale).await;
            self.alerts.submit(AlertRequest {
                monitor_id,
                monitor_name,
                transitioned_to: MonitorStatus::Stale,
            });
        }
        self.writer.submit(CheckResult {
            monitor_id,
            checked_at,
            success: false,
            latency_ms: 0,
            message: Some(message),
        });
    }

    async fn apply_result(
        &mut self,
        monitor_id: u64,
        success: bool,
        message: Option<String>,
        latency_ms: u64,
    ) {
        // The monitor may have been deleted between dispatch and this
        // result landing — resync will have already dropped it, and there
        // is nothing left to update.
        let Some(state) = self.registry.get_mut(&monitor_id) else {
            return;
        };

        if let Some(new_status) = state.flap.observe(success, state.monitor.status) {
            state.monitor.status = new_status;
            let monitor_name = state.monitor.name.clone();
            self.persist_status(monitor_id, new_status).await;
            self.alerts.submit(AlertRequest {
                monitor_id,
                monitor_name,
                transitioned_to: new_status,
            });
        }

        let result = CheckResult {
            monitor_id,
            checked_at: now_unix_secs(),
            success,
            latency_ms,
            message,
        };
        self.writer.submit(result.clone());
        // No subscribers is not an error (Phase 7 wires up WebSocket fan-out).
        let _ = self.results_tx.send(result);
    }

    async fn persist_status(&self, monitor_id: u64, status: MonitorStatus) {
        if let Err(source) = self.store.set_monitor_status(monitor_id, status).await {
            tracing::warn!(monitor_id, error = %source, "engine: failed to persist status transition");
        }
    }
}

/// Computes a monitor's next dispatch deadline anchored to its *previous*
/// deadline plus `interval`, not to `now + interval` (§6.3 point 2 / §11.5)
/// — a probe that took 3s doesn't push every future check 3s later. Only
/// falls back to `now + interval` when the monitor has fallen behind by a
/// full interval or more (the schedule would otherwise try to burst out
/// catch-up checks rather than settle back onto a steady cadence). Pure and
/// `Instant`-only (monotonic, never wall-clock) so it's directly testable
/// without pausing/advancing any real clock.
fn advance_deadline(previous_deadline: Instant, interval: Duration, now: Instant) -> Instant {
    let candidate = previous_deadline + interval;
    if candidate <= now {
        now + interval
    } else {
        candidate
    }
}

/// `<cluster>/<namespace>/<resource-name>` — the target-string convention
/// for `K8s*` monitors (DESIGN.md §5.1 left this "resolved by whichever
/// crate consumes it"; this is that resolution, made here rather than in
/// `collector-kubernetes` so the factory trait stays in terms of plain
/// cluster/namespace/name instead of a string format it has to parse).
fn parse_k8s_target(target: &str) -> Option<(&str, &str, &str)> {
    let mut parts = target.splitn(3, '/');
    let cluster = parts.next()?;
    let namespace = parts.next()?;
    let name = parts.next()?;
    if cluster.is_empty() || namespace.is_empty() || name.is_empty() {
        return None;
    }
    Some((cluster, namespace, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §7.3/§6.2 gate, mirroring `IngestHandle`'s own overflow test
    /// (`crate::tests::ingest_submit_drops_and_logs_without_blocking_when_queue_is_full`)
    /// but for the inverted pull direction: a slow/offline agent's queue
    /// drops its oldest pending assignment on overflow rather than growing
    /// without bound, and one agent falling behind never touches another's
    /// queue.
    #[test]
    fn assignment_queue_drops_oldest_on_overflow_and_is_per_agent() {
        let handle = AssignmentHandle::new(1);

        handle.enqueue(
            1,
            Assignment {
                monitor_id: 100,
                target: "a".to_string(),
                kind: MonitorKind::Http,
            },
        );
        handle.enqueue(
            1,
            Assignment {
                monitor_id: 200,
                target: "b".to_string(),
                kind: MonitorKind::Http,
            },
        );
        handle.enqueue(
            2,
            Assignment {
                monitor_id: 300,
                target: "c".to_string(),
                kind: MonitorKind::Tcp,
            },
        );

        let for_agent_1 = handle.drain(1);
        assert_eq!(
            for_agent_1.len(),
            1,
            "capacity 1: the second enqueue must have dropped the first, not grown the queue"
        );
        assert_eq!(
            for_agent_1[0].monitor_id, 200,
            "the oldest (100) must be dropped, not the newest"
        );

        let for_agent_2 = handle.drain(2);
        assert_eq!(
            for_agent_2.len(),
            1,
            "agent 1 overflowing must never affect agent 2's own queue"
        );
        assert_eq!(for_agent_2[0].monitor_id, 300);
    }

    #[test]
    fn draining_an_agent_with_nothing_queued_is_empty_not_an_error() {
        let handle = AssignmentHandle::new(4);
        assert!(handle.drain(999).is_empty());
    }

    /// §6.3/§11.5: deadlines are computed purely from monotonic `Instant`
    /// arithmetic — this test never touches wall-clock time at all, which
    /// is itself the guarantee (a `SystemTime`-based scheduler couldn't be
    /// tested this way; a clock step mid-test would change the answer).
    #[test]
    fn on_schedule_deadline_anchors_to_previous_deadline_not_to_now() {
        let previous_deadline = Instant::now();
        let interval = Duration::from_secs(30);
        // "now" is a few ms after the deadline — a probe that ran quickly.
        let now = previous_deadline + Duration::from_millis(5);

        let next = advance_deadline(previous_deadline, interval, now);

        assert_eq!(next, previous_deadline + interval);
    }

    #[test]
    fn slow_probe_does_not_shift_the_schedule() {
        let previous_deadline = Instant::now();
        let interval = Duration::from_secs(30);
        // The probe itself took 3s — still well inside the next interval.
        let now = previous_deadline + Duration::from_secs(3);

        let next = advance_deadline(previous_deadline, interval, now);

        // Anchored to the deadline that was due, not to when we got around
        // to processing it.
        assert_eq!(next, previous_deadline + interval);
    }

    #[test]
    fn falling_a_full_interval_behind_resets_rather_than_bursts() {
        let previous_deadline = Instant::now();
        let interval = Duration::from_secs(30);
        // Fell behind by more than a full interval (e.g. the scheduler
        // itself was starved for a while).
        let now = previous_deadline + Duration::from_secs(65);

        let next = advance_deadline(previous_deadline, interval, now);

        // Falls back to now + interval, not previous_deadline + interval
        // (which would already be in the past) or a burst of catch-up ticks.
        assert_eq!(next, now + interval);
        assert!(next > now);
    }

    #[test]
    fn k8s_target_parses_three_slash_separated_parts() {
        assert_eq!(
            parse_k8s_target("prod/default/api"),
            Some(("prod", "default", "api"))
        );
    }

    #[test]
    fn k8s_target_rejects_missing_parts() {
        assert_eq!(parse_k8s_target("prod/default"), None);
        assert_eq!(parse_k8s_target("prod//api"), None);
        assert_eq!(parse_k8s_target(""), None);
    }

    #[test]
    fn k8s_target_name_may_itself_contain_slashes() {
        // splitn(3, ..) leaves any further '/' in the resource-name part.
        assert_eq!(
            parse_k8s_target("prod/default/api/v2"),
            Some(("prod", "default", "api/v2"))
        );
    }

    /// A `Store` that only implements what this module's tests actually
    /// exercise (`set_monitor_status`) — everything else `unimplemented!()`,
    /// same convention as `watchdog.rs`'s own `FakeStore`.
    #[derive(Default)]
    struct FakeStore {
        statuses: Mutex<Vec<(u64, MonitorStatus)>>,
    }

    #[async_trait::async_trait]
    impl Store for FakeStore {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn health_check(&self) -> Result<(), monitra_provider::ProviderError> {
            Ok(())
        }
        async fn insert_monitor(
            &self,
            _monitor: Monitor,
        ) -> Result<Monitor, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn get_monitor(
            &self,
            _id: u64,
        ) -> Result<Option<Monitor>, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn list_monitors(&self) -> Result<Vec<Monitor>, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn update_monitor(
            &self,
            _id: u64,
            _name: Option<String>,
            _target: Option<String>,
            _interval_secs: Option<u64>,
            _agent_id: Option<u64>,
        ) -> Result<(), monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn set_monitor_status(
            &self,
            id: u64,
            status: MonitorStatus,
        ) -> Result<(), monitra_provider::ProviderError> {
            self.statuses.lock().unwrap().push((id, status));
            Ok(())
        }
        async fn delete_monitor(&self, _id: u64) -> Result<(), monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn insert_check_results(
            &self,
            _results: &[CheckResult],
        ) -> Result<(), monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn list_check_results(
            &self,
            _monitor_id: u64,
            _since: Option<u64>,
        ) -> Result<Vec<CheckResult>, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn prune_check_results_older_than(
            &self,
            _cutoff_unix_secs: u64,
        ) -> Result<u64, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn upsert_agent(
            &self,
            _agent: monitra_models::Agent,
        ) -> Result<monitra_models::Agent, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn heartbeat_agent(
            &self,
            _id: u64,
            _at_unix_secs: u64,
        ) -> Result<(), monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn get_agent(
            &self,
            _id: u64,
        ) -> Result<Option<monitra_models::Agent>, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn list_agents(
            &self,
        ) -> Result<Vec<monitra_models::Agent>, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn delete_agent(&self, _id: u64) -> Result<(), monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn insert_alert_event(
            &self,
            _event: monitra_models::AlertEvent,
        ) -> Result<monitra_models::AlertEvent, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn list_alert_events(
            &self,
            _monitor_id: u64,
        ) -> Result<Vec<monitra_models::AlertEvent>, monitra_provider::ProviderError> {
            unimplemented!()
        }
        async fn list_all_alert_events(
            &self,
        ) -> Result<Vec<monitra_models::AlertEvent>, monitra_provider::ProviderError> {
            unimplemented!()
        }
    }

    fn test_scheduler(store: Arc<dyn Store>) -> (Scheduler, mpsc::Receiver<CheckResult>) {
        let (writer, writer_rx) = Writer::test_handle(8);
        let (alerts, _alerts_rx) = Alerts::test_handle(8);
        let (results_tx, _results_rx) = broadcast::channel(8);
        let (_push_tx, push_rx) = mpsc::channel(1);
        let probers = Arc::new(Probers::new(monitra_probe::IcmpProber::new()));
        let scheduler = Scheduler::new(
            store,
            probers,
            None,
            writer,
            results_tx,
            alerts,
            push_rx,
            AssignmentHandle::new(1),
            SchedulerConfig::default(),
        );
        (scheduler, writer_rx)
    }

    /// §7.2/§11.6 gate: a probe task that panics (instead of returning an
    /// `Outcome`) must not vanish silently — before this fix, `join_next`'s
    /// `Err` case was dropped with no log and no recorded result at all.
    /// It must land exactly where a genuine "our side, not the target"
    /// failure already lands (`Unavailable` → `Stale`, §5.1/§11.3), not
    /// `Down`, and the daemon must keep running (proven implicitly: this
    /// test observes the scheduler's state *after* the panic with no
    /// special recovery code at the call site).
    #[tokio::test]
    async fn a_panicked_probe_task_is_recorded_as_stale_not_dropped() {
        let store = Arc::new(FakeStore::default());
        let (mut scheduler, mut writer_rx) = test_scheduler(Arc::clone(&store) as Arc<dyn Store>);

        let monitor_id = 1;
        scheduler.registry.insert(
            monitor_id,
            MonitorState {
                monitor: Monitor {
                    id: monitor_id,
                    name: "panicky".to_string(),
                    target: "http://example.invalid".to_string(),
                    kind: MonitorKind::Http,
                    interval_secs: 30,
                    status: MonitorStatus::Up,
                    agent_id: None,
                },
                next_check_at: Instant::now(),
                flap: FlapState::default(),
            },
        );

        let mut tasks: JoinSet<Outcome> = JoinSet::new();
        let mut owners: HashMap<TaskId, TaskOwner> = HashMap::new();
        let handle = tasks.spawn(async { panic!("simulated probe panic") });
        owners.insert(handle.id(), TaskOwner::Network(monitor_id));

        let joined = tasks
            .join_next_with_id()
            .await
            .expect("the spawned task must produce exactly one join result");

        scheduler.handle_joined(joined, &mut owners).await;

        assert!(
            owners.is_empty(),
            "the owner entry must be cleaned up whether the task succeeded or panicked"
        );
        assert_eq!(
            scheduler.registry.get(&monitor_id).unwrap().monitor.status,
            MonitorStatus::Stale,
            "an internal failure must read as Stale, never Down (P1/§11.3)"
        );
        assert_eq!(
            store.statuses.lock().unwrap().as_slice(),
            &[(monitor_id, MonitorStatus::Stale)]
        );

        let result = writer_rx
            .try_recv()
            .expect("a CheckResult must still be recorded, not silently dropped");
        assert!(!result.success);
        assert!(
            result.message.unwrap().contains("simulated probe panic"),
            "the panic payload should be preserved in the recorded message"
        );
    }
}
