//! Agent-liveness watchdog (DESIGN.md §4 `engine`, §5.1, ADR-008).
//!
//! Runs on its own timer, independent of the scheduler tick (§3.3's probe
//! loop and this loop are separate concerns: one drives checks, the other
//! watches whether the *feeds* those checks depend on are still alive).
//! When an `Agent`'s heartbeat goes quiet past `heartbeat_timeout`, every
//! `Monitor` referencing it (`Monitor.agent_id`) moves to `Stale` — never
//! `Down` — because "the agent went silent" and "the target is down" are
//! different failure signals (§5.1) that must never be collapsed into one.

use std::sync::Arc;
use std::time::Duration;

use monitra_models::MonitorStatus;
use monitra_provider::Store;
use tokio::sync::watch;

use crate::alerts::{AlertRequest, Alerts};
use crate::clock::now_unix_secs;

#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    pub check_interval: Duration,
    pub heartbeat_timeout: Duration,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(15),
            heartbeat_timeout: Duration::from_secs(90),
        }
    }
}

pub async fn run(
    store: Arc<dyn Store>,
    alerts: Alerts,
    config: WatchdogConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(config.check_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = ticker.tick() => {
                sweep(&store, &alerts, config.heartbeat_timeout).await;
            }
        }
    }
}

async fn sweep(store: &Arc<dyn Store>, alerts: &Alerts, heartbeat_timeout: Duration) {
    let agents = match store.list_agents().await {
        Ok(agents) => agents,
        Err(source) => {
            tracing::warn!(error = %source, "engine: watchdog could not list agents, skipping this sweep");
            return;
        }
    };

    let now = now_unix_secs();
    let stale_agent_ids: std::collections::HashSet<u64> = agents
        .iter()
        .filter(|agent| now.saturating_sub(agent.last_heartbeat_at) > heartbeat_timeout.as_secs())
        .map(|agent| agent.id)
        .collect();
    if stale_agent_ids.is_empty() {
        return;
    }

    let monitors = match store.list_monitors().await {
        Ok(monitors) => monitors,
        Err(source) => {
            tracing::warn!(error = %source, "engine: watchdog could not list monitors, skipping this sweep");
            return;
        }
    };

    for monitor in monitors {
        let Some(agent_id) = monitor.agent_id else {
            continue;
        };
        if !stale_agent_ids.contains(&agent_id) {
            continue;
        }
        // Paused stays paused — the user's own intent, not something the
        // watchdog should override — and already-Stale needs no rewrite.
        if matches!(monitor.status, MonitorStatus::Paused | MonitorStatus::Stale) {
            continue;
        }
        if let Err(source) = store
            .set_monitor_status(monitor.id, MonitorStatus::Stale)
            .await
        {
            tracing::warn!(
                monitor_id = monitor.id,
                agent_id,
                error = %source,
                "engine: watchdog failed to mark monitor stale"
            );
        } else {
            tracing::warn!(
                monitor_id = monitor.id,
                agent_id,
                "engine: agent heartbeat timed out, monitor marked stale"
            );
            alerts.submit(AlertRequest {
                monitor_id: monitor.id,
                monitor_name: monitor.name.clone(),
                transitioned_to: MonitorStatus::Stale,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use async_trait::async_trait;
    use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorKind};
    use monitra_provider::{ProviderCategory, ProviderError};

    use super::*;

    #[derive(Default)]
    struct FakeStore {
        agents: Mutex<Vec<Agent>>,
        monitors: Mutex<Vec<Monitor>>,
        next_id: AtomicU64,
    }

    impl FakeStore {
        fn with(agents: Vec<Agent>, monitors: Vec<Monitor>) -> Self {
            Self {
                agents: Mutex::new(agents),
                monitors: Mutex::new(monitors),
                next_id: AtomicU64::new(1),
            }
        }
    }

    #[async_trait]
    impl Store for FakeStore {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn health_check(&self) -> Result<(), ProviderError> {
            Ok(())
        }
        async fn insert_monitor(&self, monitor: Monitor) -> Result<Monitor, ProviderError> {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let monitor = Monitor { id, ..monitor };
            self.monitors.lock().unwrap().push(monitor.clone());
            Ok(monitor)
        }
        async fn get_monitor(&self, id: u64) -> Result<Option<Monitor>, ProviderError> {
            Ok(self
                .monitors
                .lock()
                .unwrap()
                .iter()
                .find(|m| m.id == id)
                .cloned())
        }
        async fn list_monitors(&self) -> Result<Vec<Monitor>, ProviderError> {
            Ok(self.monitors.lock().unwrap().clone())
        }
        async fn update_monitor(
            &self,
            _id: u64,
            _name: Option<String>,
            _target: Option<String>,
            _interval_secs: Option<u64>,
            _agent_id: Option<u64>,
        ) -> Result<(), ProviderError> {
            unimplemented!()
        }
        async fn set_monitor_status(
            &self,
            id: u64,
            status: MonitorStatus,
        ) -> Result<(), ProviderError> {
            let mut monitors = self.monitors.lock().unwrap();
            let monitor =
                monitors
                    .iter_mut()
                    .find(|m| m.id == id)
                    .ok_or(ProviderError::NotFound {
                        category: ProviderCategory::Store,
                        entity: "monitor",
                        id,
                    })?;
            monitor.status = status;
            Ok(())
        }
        async fn delete_monitor(&self, _id: u64) -> Result<(), ProviderError> {
            unimplemented!()
        }
        async fn insert_check_results(
            &self,
            _results: &[CheckResult],
        ) -> Result<(), ProviderError> {
            unimplemented!()
        }
        async fn list_check_results(
            &self,
            _monitor_id: u64,
            _since: Option<u64>,
        ) -> Result<Vec<CheckResult>, ProviderError> {
            unimplemented!()
        }
        async fn prune_check_results_older_than(
            &self,
            _cutoff_unix_secs: u64,
        ) -> Result<u64, ProviderError> {
            unimplemented!()
        }
        async fn upsert_agent(&self, _agent: Agent) -> Result<Agent, ProviderError> {
            unimplemented!()
        }
        async fn heartbeat_agent(&self, _id: u64, _at_unix_secs: u64) -> Result<(), ProviderError> {
            unimplemented!()
        }
        async fn get_agent(&self, id: u64) -> Result<Option<Agent>, ProviderError> {
            Ok(self
                .agents
                .lock()
                .unwrap()
                .iter()
                .find(|a| a.id == id)
                .cloned())
        }
        async fn list_agents(&self) -> Result<Vec<Agent>, ProviderError> {
            Ok(self.agents.lock().unwrap().clone())
        }
        async fn delete_agent(&self, _id: u64) -> Result<(), ProviderError> {
            unimplemented!()
        }
        async fn insert_alert_event(
            &self,
            _event: AlertEvent,
        ) -> Result<AlertEvent, ProviderError> {
            unimplemented!()
        }
        async fn list_alert_events(
            &self,
            _monitor_id: u64,
        ) -> Result<Vec<AlertEvent>, ProviderError> {
            unimplemented!()
        }
        async fn list_all_alert_events(&self) -> Result<Vec<AlertEvent>, ProviderError> {
            unimplemented!()
        }
    }

    fn monitor(id: u64, agent_id: Option<u64>, status: MonitorStatus) -> Monitor {
        Monitor {
            id,
            name: "m".to_string(),
            target: "edge-1:disk".to_string(),
            kind: MonitorKind::HostAgentCheck,
            interval_secs: 30,
            status,
            agent_id,
        }
    }

    #[tokio::test]
    async fn stale_heartbeat_demotes_dependent_monitors_to_stale_not_down() {
        let now = now_unix_secs();
        let agent = Agent {
            id: 1,
            name: "edge-1".to_string(),
            last_heartbeat_at: now - 1000,
            scope: "host:edge-1".to_string(),
            token: "token".to_string(),
        };
        let store = Arc::new(FakeStore::with(
            vec![agent],
            vec![monitor(1, Some(1), MonitorStatus::Up)],
        )) as Arc<dyn Store>;

        let (alerts, _alerts_rx) = Alerts::test_handle(8);
        sweep(&store, &alerts, Duration::from_secs(90)).await;

        let updated = store.get_monitor(1).await.unwrap().unwrap();
        assert_eq!(updated.status, MonitorStatus::Stale);
    }

    #[tokio::test]
    async fn fresh_heartbeat_leaves_monitors_untouched() {
        let now = now_unix_secs();
        let agent = Agent {
            id: 1,
            name: "edge-1".to_string(),
            last_heartbeat_at: now,
            scope: "host:edge-1".to_string(),
            token: "token".to_string(),
        };
        let store = Arc::new(FakeStore::with(
            vec![agent],
            vec![monitor(1, Some(1), MonitorStatus::Up)],
        )) as Arc<dyn Store>;

        let (alerts, _alerts_rx) = Alerts::test_handle(8);
        sweep(&store, &alerts, Duration::from_secs(90)).await;

        let updated = store.get_monitor(1).await.unwrap().unwrap();
        assert_eq!(updated.status, MonitorStatus::Up);
    }

    #[tokio::test]
    async fn paused_monitors_are_never_overridden() {
        let now = now_unix_secs();
        let agent = Agent {
            id: 1,
            name: "edge-1".to_string(),
            last_heartbeat_at: now - 1000,
            scope: "host:edge-1".to_string(),
            token: "token".to_string(),
        };
        let store = Arc::new(FakeStore::with(
            vec![agent],
            vec![monitor(1, Some(1), MonitorStatus::Paused)],
        )) as Arc<dyn Store>;

        let (alerts, _alerts_rx) = Alerts::test_handle(8);
        sweep(&store, &alerts, Duration::from_secs(90)).await;

        let updated = store.get_monitor(1).await.unwrap().unwrap();
        assert_eq!(updated.status, MonitorStatus::Paused);
    }

    #[tokio::test]
    async fn monitors_with_no_agent_are_unaffected() {
        let now = now_unix_secs();
        let agent = Agent {
            id: 1,
            name: "edge-1".to_string(),
            last_heartbeat_at: now - 1000,
            scope: "host:edge-1".to_string(),
            token: "token".to_string(),
        };
        let store = Arc::new(FakeStore::with(
            vec![agent],
            vec![monitor(1, None, MonitorStatus::Up)],
        )) as Arc<dyn Store>;

        let (alerts, _alerts_rx) = Alerts::test_handle(8);
        sweep(&store, &alerts, Duration::from_secs(90)).await;

        let updated = store.get_monitor(1).await.unwrap().unwrap();
        assert_eq!(updated.status, MonitorStatus::Up);
    }
}
