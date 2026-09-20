//! A real, stateful `Store` implementation for `backend`'s integration
//! tests — `Vec`-backed rather than SQLite, but genuine CRUD with the same
//! `NotFound` semantics `SqliteStore` has, not a canned mock. `backend` may
//! never depend on `storage` (DESIGN.md §3.2 — enforced on dev-dependencies
//! too, `scripts/dep-check.py`), so this lives here instead of reusing
//! `storage`'s real SQLite impl.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorStatus};
use monitra_provider::{ProviderCategory, ProviderError, Store};

#[derive(Default)]
pub struct InMemoryStore {
    monitors: Mutex<Vec<Monitor>>,
    agents: Mutex<Vec<Agent>>,
    check_results: Mutex<Vec<CheckResult>>,
    alert_events: Mutex<Vec<AlertEvent>>,
    next_monitor_id: AtomicU64,
    next_agent_id: AtomicU64,
    next_alert_id: AtomicU64,
    healthy: AtomicBool,
}

impl InMemoryStore {
    pub fn new() -> Self {
        let store = Self::default();
        store.healthy.store(true, Ordering::SeqCst);
        store
    }

    /// Flips whether `health_check` succeeds — used to exercise `/health`'s
    /// "store unreachable" path deterministically. Only `tests/api.rs` uses
    /// this; `#[allow(dead_code)]` because this file is compiled fresh into
    /// every test binary that has `mod support;`, and per-binary dead-code
    /// analysis doesn't see across them.
    #[allow(dead_code)]
    pub fn set_healthy(&self, healthy: bool) {
        self.healthy.store(healthy, Ordering::SeqCst);
    }

    fn not_found(entity: &'static str, id: u64) -> ProviderError {
        ProviderError::NotFound {
            category: ProviderCategory::Store,
            entity,
            id,
        }
    }
}

#[async_trait]
impl Store for InMemoryStore {
    fn name(&self) -> &'static str {
        "in-memory-test-store"
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        if self.healthy.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(ProviderError::Unavailable {
                category: ProviderCategory::Store,
                detail: "in-memory test store: marked unhealthy".to_string(),
            })
        }
    }

    async fn insert_monitor(&self, monitor: Monitor) -> Result<Monitor, ProviderError> {
        let id = self.next_monitor_id.fetch_add(1, Ordering::SeqCst) + 1;
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
        id: u64,
        name: Option<String>,
        target: Option<String>,
        interval_secs: Option<u64>,
        agent_id: Option<u64>,
    ) -> Result<(), ProviderError> {
        let mut monitors = self.monitors.lock().unwrap();
        let monitor = monitors
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| Self::not_found("monitor", id))?;
        if let Some(name) = name {
            monitor.name = name;
        }
        if let Some(target) = target {
            monitor.target = target;
        }
        if let Some(interval_secs) = interval_secs {
            monitor.interval_secs = interval_secs;
        }
        if let Some(agent_id) = agent_id {
            monitor.agent_id = Some(agent_id);
        }
        Ok(())
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
            .ok_or_else(|| Self::not_found("monitor", id))?;
        monitor.status = status;
        Ok(())
    }

    async fn delete_monitor(&self, id: u64) -> Result<(), ProviderError> {
        let mut monitors = self.monitors.lock().unwrap();
        let before = monitors.len();
        monitors.retain(|m| m.id != id);
        if monitors.len() == before {
            return Err(Self::not_found("monitor", id));
        }
        Ok(())
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
        monitor_id: u64,
        since: Option<u64>,
    ) -> Result<Vec<CheckResult>, ProviderError> {
        let since = since.unwrap_or(0);
        Ok(self
            .check_results
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.monitor_id == monitor_id && r.checked_at >= since)
            .cloned()
            .collect())
    }

    async fn prune_check_results_older_than(
        &self,
        cutoff_unix_secs: u64,
    ) -> Result<u64, ProviderError> {
        let mut results = self.check_results.lock().unwrap();
        let before = results.len();
        results.retain(|r| r.checked_at >= cutoff_unix_secs);
        Ok((before - results.len()) as u64)
    }

    async fn upsert_agent(&self, agent: Agent) -> Result<Agent, ProviderError> {
        let mut agents = self.agents.lock().unwrap();
        if let Some(existing) = agents.iter_mut().find(|a| a.name == agent.name) {
            existing.last_heartbeat_at = agent.last_heartbeat_at;
            existing.scope = agent.scope.clone();
            existing.token = agent.token.clone();
            return Ok(existing.clone());
        }
        let id = self.next_agent_id.fetch_add(1, Ordering::SeqCst) + 1;
        let agent = Agent { id, ..agent };
        agents.push(agent.clone());
        Ok(agent)
    }

    async fn heartbeat_agent(&self, id: u64, at_unix_secs: u64) -> Result<(), ProviderError> {
        let mut agents = self.agents.lock().unwrap();
        let agent = agents
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| Self::not_found("agent", id))?;
        agent.last_heartbeat_at = at_unix_secs;
        Ok(())
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

    async fn delete_agent(&self, id: u64) -> Result<(), ProviderError> {
        let mut agents = self.agents.lock().unwrap();
        let before = agents.len();
        agents.retain(|a| a.id != id);
        if agents.len() == before {
            return Err(Self::not_found("agent", id));
        }
        Ok(())
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
}
