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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant as StdInstant};

use monitra_models::{CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_provider::{Collector, CollectorStatus, K8sCollectorFactory, Store};
use tokio::sync::{Semaphore, broadcast, mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::alerts::{AlertRequest, Alerts};
use crate::clock::now_unix_secs;
use crate::flap::FlapState;
use crate::probe::{NetworkProbeKind, ProbeOutcome, Probers};
use crate::writer::Writer;

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

pub struct Scheduler {
    store: Arc<dyn Store>,
    probers: Arc<Probers>,
    k8s_factory: Option<Arc<dyn K8sCollectorFactory>>,
    semaphore: Arc<Semaphore>,
    writer: Writer,
    results_tx: broadcast::Sender<CheckResult>,
    alerts: Alerts,
    push_rx: mpsc::Receiver<PushedResult>,
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
                    self.dispatch_due(&mut tasks);
                }
                Some(joined) = tasks.join_next(), if !tasks.is_empty() => {
                    if let Ok(outcome) = joined {
                        self.handle_outcome(outcome).await;
                    }
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
            while let Some(joined) = tasks.join_next().await {
                if let Ok(outcome) = joined {
                    self.handle_outcome(outcome).await;
                }
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

    fn dispatch_due(&mut self, tasks: &mut JoinSet<Outcome>) {
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
                Ok(kind) => self.dispatch_network(monitor_id, kind, &monitor.target, tasks),
                Err(()) if monitor.kind == MonitorKind::HostAgentCheck => {
                    // Pushed by agents (Phase 8) — never scheduler-dispatched.
                }
                Err(()) => self.dispatch_k8s(monitor_id, &monitor, tasks),
            }
        }
    }

    fn dispatch_network(
        &self,
        monitor_id: u64,
        kind: NetworkProbeKind,
        target: &str,
        tasks: &mut JoinSet<Outcome>,
    ) {
        let probers = Arc::clone(&self.probers);
        let semaphore = Arc::clone(&self.semaphore);
        let timeout = self.config.probe_timeout;
        let target = target.to_string();
        tasks.spawn(async move {
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
    }

    fn dispatch_k8s(&mut self, monitor_id: u64, monitor: &Monitor, tasks: &mut JoinSet<Outcome>) {
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
            tasks.spawn(async move {
                Outcome::Collector {
                    monitor_id,
                    status: CollectorStatus::Unknown {
                        reason: "collector unavailable".to_string(),
                    },
                    latency_ms: 0,
                }
            });
            return;
        };

        let semaphore = Arc::clone(&self.semaphore);
        tasks.spawn(async move {
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
}
