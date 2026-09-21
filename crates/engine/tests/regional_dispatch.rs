//! Phase 11 gate (ADR-011): the scheduler must route an `agent_id`-linked
//! network-probe `Monitor` to that agent's [`AssignmentHandle`] queue
//! instead of probing it centrally — this closes the pre-Phase-11 bug where
//! `dispatch_due` ignored `Monitor.agent_id` entirely for `Http`/`Tcp`/
//! `Icmp` kinds. A monitor with no `agent_id` must still be dispatched
//! centrally exactly as before, so the fix doesn't silently stop probing
//! the common case.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use monitra_engine::{EngineConfig, EngineDeps, EngineHandle, SchedulerConfig};
use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_provider::{Notifier, ProviderError, RetryingNotifier, Store};

#[derive(Default)]
struct FakeStore {
    monitors: Mutex<Vec<Monitor>>,
    check_results: Mutex<Vec<CheckResult>>,
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
        if let Some(monitor) = self
            .monitors
            .lock()
            .unwrap()
            .iter_mut()
            .find(|m| m.id == id)
        {
            monitor.status = status;
        }
        Ok(())
    }
    async fn delete_monitor(&self, _id: u64) -> Result<(), ProviderError> {
        unimplemented!()
    }
    async fn insert_check_results(&self, results: &[CheckResult]) -> Result<(), ProviderError> {
        self.check_results
            .lock()
            .unwrap()
            .extend_from_slice(results);
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
        Ok(event)
    }
    async fn list_alert_events(&self, _monitor_id: u64) -> Result<Vec<AlertEvent>, ProviderError> {
        Ok(Vec::new())
    }
    async fn list_all_alert_events(&self) -> Result<Vec<AlertEvent>, ProviderError> {
        Ok(Vec::new())
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
async fn agent_linked_network_monitor_is_assigned_not_dispatched_centrally() {
    // A real local listener so the *centrally*-dispatched monitor (no
    // `agent_id`) gets a genuine Success, proving that path still works
    // unchanged.
    let central_target = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local listener");
    let central_addr = central_target.local_addr().expect("local addr");
    tokio::spawn(async move {
        loop {
            if central_target.accept().await.is_err() {
                break;
            }
        }
    });

    let store = Arc::new(FakeStore::default());
    {
        let mut monitors = store.monitors.lock().unwrap();
        monitors.push(Monitor {
            id: 1,
            name: "regional".to_string(),
            target: "127.0.0.1:1".to_string(), // never actually dialed by the engine
            kind: MonitorKind::Tcp,
            interval_secs: 1,
            status: MonitorStatus::Pending,
            agent_id: Some(42),
        });
        monitors.push(Monitor {
            id: 2,
            name: "central".to_string(),
            target: central_addr.to_string(),
            kind: MonitorKind::Tcp,
            interval_secs: 1,
            status: MonitorStatus::Pending,
            agent_id: None,
        });
    }

    let notifier = Arc::new(RetryingNotifier::new(Arc::new(AlwaysHealthyNotifier), 8));
    let engine = EngineHandle::start(
        EngineDeps {
            store: Arc::clone(&store) as Arc<dyn Store>,
            k8s_factory: None,
            notifier,
        },
        EngineConfig {
            scheduler: SchedulerConfig {
                resync_interval: Duration::from_millis(20),
                ..SchedulerConfig::default()
            },
            ..EngineConfig::default()
        },
    );

    let assignments = engine.assignment_handle();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let drained = loop {
        let drained = assignments.drain(42);
        if !drained.is_empty() {
            break drained;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the agent-linked monitor was never enqueued as an assignment \
             — dispatch_due must still be probing it centrally"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].monitor_id, 1);
    assert_eq!(drained[0].kind, MonitorKind::Tcp);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if !store.check_results.lock().unwrap().is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the agent-less monitor must still be dispatched and probed centrally"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let results = store.check_results.lock().unwrap().clone();
    assert!(
        results.iter().all(|r| r.monitor_id == 2),
        "only the centrally-dispatched monitor should ever produce a directly-written \
         CheckResult here — the regional one must never be probed by the engine itself, got {results:?}"
    );
    assert!(
        results.iter().any(|r| r.success),
        "the central monitor's real TCP probe against a live local listener must succeed"
    );

    engine.shutdown().await;
}
