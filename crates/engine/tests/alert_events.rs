//! Phase 7 gate: an `AlertEvent` row is created for a real status
//! transition, driven end to end through a running `EngineHandle` (not
//! just `alerts.rs`'s own unit tests against the `Alerts` task in
//! isolation) — this is what actually proves the scheduler/watchdog wiring
//! added at Phase 7, not only the alert-delivery mechanism itself.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use monitra_engine::{
    EngineConfig, EngineDeps, EngineHandle, ProbeOutcome, PushedResult, SchedulerConfig,
};
use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_provider::{Notifier, ProviderCategory, ProviderError, RetryingNotifier, Store};

#[derive(Default)]
struct FakeStore {
    monitors: Mutex<Vec<Monitor>>,
    alert_events: Mutex<Vec<AlertEvent>>,
    next_alert_id: AtomicU64,
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
        let monitor = monitors
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
    async fn insert_check_results(&self, _results: &[CheckResult]) -> Result<(), ProviderError> {
        Ok(())
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
    async fn get_agent(&self, _id: u64) -> Result<Option<Agent>, ProviderError> {
        unimplemented!()
    }
    async fn list_agents(&self) -> Result<Vec<Agent>, ProviderError> {
        Ok(Vec::new())
    }
    async fn delete_agent(&self, _id: u64) -> Result<(), ProviderError> {
        unimplemented!()
    }
    async fn insert_alert_event(&self, event: AlertEvent) -> Result<AlertEvent, ProviderError> {
        let id = self.next_alert_id.fetch_add(1, Ordering::SeqCst) + 1;
        let event = AlertEvent { id, ..event };
        self.alert_events.lock().unwrap().push(event.clone());
        Ok(event)
    }
    async fn list_alert_events(&self, monitor_id: u64) -> Result<Vec<AlertEvent>, ProviderError> {
        Ok(self
            .alert_events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.monitor_id == monitor_id)
            .cloned()
            .collect())
    }
    async fn list_all_alert_events(&self) -> Result<Vec<AlertEvent>, ProviderError> {
        Ok(self.alert_events.lock().unwrap().clone())
    }
}

struct AlwaysHealthyNotifier;

#[async_trait]
impl Notifier for AlwaysHealthyNotifier {
    fn name(&self) -> &'static str {
        "test"
    }
    async fn notify(&self, _message: &str) -> Result<(), ProviderError> {
        Ok(())
    }
}

#[tokio::test]
async fn a_real_status_transition_persists_an_alert_event() {
    let store = Arc::new(FakeStore::default());
    store.monitors.lock().unwrap().push(Monitor {
        id: 1,
        name: "host-check".to_string(),
        target: "edge-1:disk".to_string(),
        kind: MonitorKind::HostAgentCheck,
        interval_secs: 30,
        status: MonitorStatus::Pending,
        agent_id: None,
    });

    let notifier = Arc::new(RetryingNotifier::new(Arc::new(AlwaysHealthyNotifier), 8));
    let engine = EngineHandle::start(
        EngineDeps {
            store: Arc::clone(&store) as Arc<dyn Store>,
            k8s_factory: None,
            notifier,
        },
        EngineConfig {
            // Fast resync so the scheduler picks the seeded monitor up
            // (and the ingest path's `apply_result` finds it in registry)
            // well within the test's timeout.
            scheduler: SchedulerConfig {
                resync_interval: Duration::from_millis(20),
                ..SchedulerConfig::default()
            },
            ..EngineConfig::default()
        },
    );

    // Let the first resync land before pushing.
    tokio::time::sleep(Duration::from_millis(60)).await;

    let ingest = engine.ingest_handle();
    // Flap damping needs two consecutive failures to transition Pending -> Down.
    ingest.submit(PushedResult {
        monitor_id: 1,
        outcome: ProbeOutcome::Failure {
            message: "disk full".to_string(),
        },
    });
    ingest.submit(PushedResult {
        monitor_id: 1,
        outcome: ProbeOutcome::Failure {
            message: "disk full".to_string(),
        },
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if !store.alert_events.lock().unwrap().is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no AlertEvent was persisted within the timeout"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    engine.shutdown().await;

    let events = store.alert_events.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].monitor_id, 1);
    assert_eq!(events[0].transitioned_to, MonitorStatus::Down);
    assert_eq!(events[0].delivery_outcome, "sent");

    let monitor = store.get_monitor(1).await.unwrap().unwrap();
    assert_eq!(monitor.status, MonitorStatus::Down);
}
