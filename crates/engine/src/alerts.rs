//! `AlertEvent` emission (DESIGN.md §5.1, §7.2, Phase 7): on every
//! engine-driven status transition, attempts delivery via the configured
//! `Notifier` (through the bounded-queue-with-retry `RetryingNotifier`,
//! `monitra-provider`) and persists the outcome as an `AlertEvent` row, so
//! "did this alert actually go out" is answerable from alert history alone,
//! not just logs.
//!
//! Bounded (§7.3): a transition never blocks on this — `submit` drops the
//! newest request and logs loudly on overflow, the same policy `writer.rs`
//! uses for check results. Backoff redelivery of anything still queued in
//! the `RetryingNotifier` happens on a fixed interval here, not per-message
//! exponential backoff — the simplest thing that satisfies §4.1's "bounded
//! retry and backoff" without speculative complexity (P5).

use std::sync::Arc;
use std::time::Duration;

use monitra_models::{AlertEvent, MonitorStatus};
use monitra_provider::{DeliveryOutcome, RetryingNotifier, Store};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::clock::now_unix_secs;

#[derive(Debug, Clone)]
pub struct AlertConfig {
    pub queue_capacity: usize,
    pub retry_interval: Duration,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 256,
            retry_interval: Duration::from_secs(30),
        }
    }
}

pub struct AlertRequest {
    pub monitor_id: u64,
    pub monitor_name: String,
    pub transitioned_to: MonitorStatus,
}

#[derive(Clone)]
pub struct Alerts {
    tx: mpsc::Sender<AlertRequest>,
}

impl Alerts {
    pub fn spawn(
        store: Arc<dyn Store>,
        notifier: Arc<RetryingNotifier>,
        config: AlertConfig,
    ) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(config.queue_capacity.max(1));
        let handle = tokio::spawn(run(store, notifier, rx, config.retry_interval));
        (Self { tx }, handle)
    }

    /// A handle with nothing consuming it — for other modules' tests
    /// (`watchdog.rs`) that need an `Alerts` to construct their subject but
    /// don't care what happens to the requests. Draining `rx` (or dropping
    /// it, which just makes future `submit`s silently no-op via the bounded
    /// channel's normal drop-and-log path once full) is up to the caller.
    #[cfg(test)]
    pub(crate) fn test_handle(capacity: usize) -> (Self, mpsc::Receiver<AlertRequest>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (Self { tx }, rx)
    }

    /// Never blocks (§7.3) — drops and logs loudly on a full queue rather
    /// than back-pressuring the scheduler/watchdog transition path.
    pub fn submit(&self, request: AlertRequest) {
        let monitor_id = request.monitor_id;
        if let Err(source) = self.tx.try_send(request) {
            tracing::warn!(
                monitor_id,
                error = %source,
                "engine: alert queue full, dropping alert-event request"
            );
        }
    }
}

async fn run(
    store: Arc<dyn Store>,
    notifier: Arc<RetryingNotifier>,
    mut rx: mpsc::Receiver<AlertRequest>,
    retry_interval: Duration,
) {
    let mut ticker = tokio::time::interval(retry_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            received = rx.recv() => {
                match received {
                    Some(request) => handle_request(&store, &notifier, request).await,
                    // Every `Alerts` handle (and thus every `Sender`) has
                    // been dropped — engine shutting down.
                    None => return,
                }
            }
            _ = ticker.tick() => {
                notifier.drain_retries().await;
            }
        }
    }
}

async fn handle_request(
    store: &Arc<dyn Store>,
    notifier: &Arc<RetryingNotifier>,
    request: AlertRequest,
) {
    let message = format!(
        "monitor '{}' ({}) transitioned to {:?}",
        request.monitor_name, request.monitor_id, request.transitioned_to
    );
    let outcome = notifier.notify_recording_outcome(&message).await;
    let delivery_outcome = match outcome {
        DeliveryOutcome::Sent => "sent".to_string(),
        DeliveryOutcome::Queued => "queued for retry".to_string(),
    };

    let event = AlertEvent {
        id: 0,
        monitor_id: request.monitor_id,
        transitioned_to: request.transitioned_to,
        occurred_at: now_unix_secs(),
        sinks_attempted: notifier.inner_name().to_string(),
        delivery_outcome,
    };
    if let Err(source) = store.insert_alert_event(event).await {
        tracing::warn!(
            monitor_id = request.monitor_id,
            error = %source,
            "engine: failed to persist alert event"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use async_trait::async_trait;
    use monitra_models::{CheckResult, Monitor};
    use monitra_provider::{Notifier, ProviderCategory, ProviderError};

    use super::*;

    #[derive(Default)]
    struct FakeStore {
        alert_events: Mutex<Vec<AlertEvent>>,
        next_id: AtomicU64,
    }

    #[async_trait]
    impl Store for FakeStore {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn health_check(&self) -> Result<(), ProviderError> {
            Ok(())
        }
        async fn insert_monitor(&self, _monitor: Monitor) -> Result<Monitor, ProviderError> {
            unimplemented!()
        }
        async fn get_monitor(&self, _id: u64) -> Result<Option<Monitor>, ProviderError> {
            unimplemented!()
        }
        async fn list_monitors(&self) -> Result<Vec<Monitor>, ProviderError> {
            unimplemented!()
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
            _id: u64,
            _status: MonitorStatus,
        ) -> Result<(), ProviderError> {
            unimplemented!()
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
        async fn upsert_agent(
            &self,
            _agent: monitra_models::Agent,
        ) -> Result<monitra_models::Agent, ProviderError> {
            unimplemented!()
        }
        async fn heartbeat_agent(&self, _id: u64, _at_unix_secs: u64) -> Result<(), ProviderError> {
            unimplemented!()
        }
        async fn get_agent(
            &self,
            _id: u64,
        ) -> Result<Option<monitra_models::Agent>, ProviderError> {
            unimplemented!()
        }
        async fn list_agents(&self) -> Result<Vec<monitra_models::Agent>, ProviderError> {
            unimplemented!()
        }
        async fn delete_agent(&self, _id: u64) -> Result<(), ProviderError> {
            unimplemented!()
        }
        async fn insert_alert_event(&self, event: AlertEvent) -> Result<AlertEvent, ProviderError> {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
            let event = AlertEvent { id, ..event };
            self.alert_events.lock().unwrap().push(event.clone());
            Ok(event)
        }
        async fn list_alert_events(
            &self,
            _monitor_id: u64,
        ) -> Result<Vec<AlertEvent>, ProviderError> {
            unimplemented!()
        }
    }

    struct FakeNotifier {
        healthy: AtomicBool,
    }

    #[async_trait]
    impl Notifier for FakeNotifier {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn notify(&self, _message: &str) -> Result<(), ProviderError> {
            if self.healthy.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(ProviderError::Unavailable {
                    category: ProviderCategory::Notifier,
                    detail: "fake notifier: down".to_string(),
                })
            }
        }
    }

    fn request(monitor_id: u64, status: MonitorStatus) -> AlertRequest {
        AlertRequest {
            monitor_id,
            monitor_name: "m".to_string(),
            transitioned_to: status,
        }
    }

    #[tokio::test]
    async fn successful_delivery_records_sent_outcome() {
        let store = Arc::new(FakeStore::default());
        let notifier = Arc::new(RetryingNotifier::new(
            Arc::new(FakeNotifier {
                healthy: AtomicBool::new(true),
            }),
            8,
        ));
        let (alerts, _task) = Alerts::spawn(
            store.clone() as Arc<dyn Store>,
            notifier,
            AlertConfig {
                queue_capacity: 8,
                retry_interval: Duration::from_secs(3600),
            },
        );

        alerts.submit(request(1, MonitorStatus::Down));
        // Give the background task a beat to process.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let events = store.alert_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].monitor_id, 1);
        assert_eq!(events[0].transitioned_to, MonitorStatus::Down);
        assert_eq!(events[0].delivery_outcome, "sent");
    }

    #[tokio::test]
    async fn failed_delivery_still_records_an_alert_event_as_queued() {
        let store = Arc::new(FakeStore::default());
        let notifier = Arc::new(RetryingNotifier::new(
            Arc::new(FakeNotifier {
                healthy: AtomicBool::new(false),
            }),
            8,
        ));
        let (alerts, _task) = Alerts::spawn(
            store.clone() as Arc<dyn Store>,
            notifier,
            AlertConfig {
                queue_capacity: 8,
                retry_interval: Duration::from_secs(3600),
            },
        );

        alerts.submit(request(2, MonitorStatus::Up));
        tokio::time::sleep(Duration::from_millis(50)).await;

        let events = store.alert_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delivery_outcome, "queued for retry");
    }

    #[tokio::test]
    async fn submit_drops_and_logs_without_blocking_when_queue_is_full() {
        // A capacity-1 handle with nothing draining it, so overflow is
        // deterministic rather than racing a background task (§7.3: a full
        // queue must drop, not block).
        let (alerts, mut rx) = Alerts::test_handle(1);

        alerts.submit(request(1, MonitorStatus::Down));
        alerts.submit(request(2, MonitorStatus::Down));

        let received = rx.recv().await.expect("first request was queued");
        assert_eq!(received.monitor_id, 1);
        assert!(
            rx.try_recv().is_err(),
            "second request must have been dropped, not queued behind the first"
        );
    }
}
