//! The §4.1 per-category availability policies. Each function/type here is
//! the actual behavior a category exhibits when its configured provider is
//! unreachable — not just documentation of the rule.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::category::ProviderCategory;
use crate::error::ProviderError;
use crate::traits::{Cache, Collector, CollectorStatus, Notifier};

/// `Store`: fail fast. A configured-but-unreachable store is a hard error —
/// there is no fallback path, because silently splitting history across two
/// stores is worse than refusing to start (§4.1).
pub async fn resolve_store(
    configured: Option<&dyn crate::traits::Store>,
) -> Result<(), ProviderError> {
    match configured {
        None => Ok(()),
        Some(store) => store.health_check().await.map_err(|source| {
            tracing::error!(store = store.name(), error = %source, "configured store unreachable, refusing to start");
            ProviderError::Unavailable {
                category: ProviderCategory::Store,
                detail: format!("configured store '{}' is unreachable: {source}", store.name()),
            }
        }),
    }
}

/// `Cache`: degrade to the in-process default and log at WARN. A cache miss
/// costs latency, nothing more (§4.1) — never returns an error to its caller.
pub struct DegradingCache {
    configured: Option<Arc<dyn Cache>>,
    default: Arc<dyn Cache>,
    degraded: AtomicBool,
}

impl DegradingCache {
    pub fn new(configured: Option<Arc<dyn Cache>>, default: Arc<dyn Cache>) -> Self {
        Self {
            configured,
            default,
            degraded: AtomicBool::new(false),
        }
    }

    fn note_degradation(&self, name: &str, error: &ProviderError) {
        if !self.degraded.swap(true, Ordering::SeqCst) {
            tracing::warn!(cache = name, error = %error, "configured cache unreachable, degrading to in-process default");
        }
    }

    pub fn is_degraded(&self) -> bool {
        self.degraded.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Cache for DegradingCache {
    fn name(&self) -> &'static str {
        "degrading"
    }

    async fn get(&self, key: &str) -> Result<Option<String>, ProviderError> {
        if !self.degraded.load(Ordering::SeqCst)
            && let Some(configured) = &self.configured
        {
            match configured.get(key).await {
                Ok(value) => return Ok(value),
                Err(error) => self.note_degradation(configured.name(), &error),
            }
        }
        self.default.get(key).await
    }

    async fn set(&self, key: &str, value: &str) -> Result<(), ProviderError> {
        if !self.degraded.load(Ordering::SeqCst)
            && let Some(configured) = &self.configured
        {
            match configured.set(key, value).await {
                Ok(()) => return Ok(()),
                Err(error) => self.note_degradation(configured.name(), &error),
            }
        }
        self.default.set(key, value).await
    }
}

/// `Notifier`: bounded queue with retry, log loudly on overflow. Never blocks
/// a probe (§4.1) — `notify` always returns `Ok`, queuing on failure instead
/// of propagating it.
pub struct RetryingNotifier {
    inner: Arc<dyn Notifier>,
    queue: Mutex<VecDeque<String>>,
    capacity: usize,
}

impl RetryingNotifier {
    pub fn new(inner: Arc<dyn Notifier>, capacity: usize) -> Self {
        Self {
            inner,
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    pub fn queue_len(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .len()
    }

    fn enqueue(&self, message: String) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if queue.len() >= self.capacity {
            let dropped = queue.pop_front();
            tracing::warn!(
                notifier = self.inner.name(),
                dropped = dropped.as_deref().unwrap_or(""),
                capacity = self.capacity,
                "notifier retry queue full, dropping oldest message"
            );
        }
        queue.push_back(message);
    }

    /// Attempts to redeliver every queued message once. Messages that still
    /// fail stay queued, in order, for the next attempt.
    pub async fn drain_retries(&self) {
        let pending: Vec<String> = {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            queue.drain(..).collect()
        };
        for message in pending {
            if self.inner.notify(&message).await.is_err() {
                self.enqueue(message);
            }
        }
    }
}

#[async_trait]
impl Notifier for RetryingNotifier {
    fn name(&self) -> &'static str {
        "retrying"
    }

    async fn notify(&self, message: &str) -> Result<(), ProviderError> {
        if let Err(error) = self.inner.notify(message).await {
            tracing::warn!(notifier = self.inner.name(), error = %error, "notify failed, queuing for retry");
            self.enqueue(message.to_string());
        }
        Ok(())
    }
}

/// `Collector`: per-resource WARN + `Unknown`, never fails the daemon (§4.1).
pub async fn poll_collector_safely(collector: &dyn Collector) -> CollectorStatus {
    match collector.poll().await {
        Ok(status) => status,
        Err(error) => {
            tracing::warn!(collector = collector.name(), error = %error, "collector poll failed, marking resource unknown");
            CollectorStatus::Unknown {
                reason: error.to_string(),
            }
        }
    }
}

#[cfg(test)]
mod fakes {
    use super::*;

    pub struct FakeStore {
        pub healthy: bool,
    }

    #[async_trait]
    impl crate::traits::Store for FakeStore {
        fn name(&self) -> &'static str {
            "fake-store"
        }

        async fn health_check(&self) -> Result<(), ProviderError> {
            if self.healthy {
                Ok(())
            } else {
                Err(ProviderError::Unavailable {
                    category: ProviderCategory::Store,
                    detail: "fake store: connection refused".to_string(),
                })
            }
        }

        async fn insert_monitor(
            &self,
            _monitor: monitra_models::Monitor,
        ) -> Result<monitra_models::Monitor, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn get_monitor(
            &self,
            _id: u64,
        ) -> Result<Option<monitra_models::Monitor>, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn list_monitors(&self) -> Result<Vec<monitra_models::Monitor>, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn update_monitor(
            &self,
            _id: u64,
            _name: Option<String>,
            _target: Option<String>,
            _interval_secs: Option<u64>,
        ) -> Result<(), ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn set_monitor_status(
            &self,
            _id: u64,
            _status: monitra_models::MonitorStatus,
        ) -> Result<(), ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn delete_monitor(&self, _id: u64) -> Result<(), ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn insert_check_results(
            &self,
            _results: &[monitra_models::CheckResult],
        ) -> Result<(), ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn list_check_results(
            &self,
            _monitor_id: u64,
            _since: Option<u64>,
        ) -> Result<Vec<monitra_models::CheckResult>, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn prune_check_results_older_than(
            &self,
            _cutoff_unix_secs: u64,
        ) -> Result<u64, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn upsert_agent(
            &self,
            _agent: monitra_models::Agent,
        ) -> Result<monitra_models::Agent, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn heartbeat_agent(&self, _id: u64, _at_unix_secs: u64) -> Result<(), ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn get_agent(
            &self,
            _id: u64,
        ) -> Result<Option<monitra_models::Agent>, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn list_agents(&self) -> Result<Vec<monitra_models::Agent>, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn delete_agent(&self, _id: u64) -> Result<(), ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn insert_alert_event(
            &self,
            _event: monitra_models::AlertEvent,
        ) -> Result<monitra_models::AlertEvent, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }

        async fn list_alert_events(
            &self,
            _monitor_id: u64,
        ) -> Result<Vec<monitra_models::AlertEvent>, ProviderError> {
            unimplemented!("fake store: only health_check/resolve_store are exercised here")
        }
    }

    pub struct FakeCache {
        pub healthy: bool,
    }

    #[async_trait]
    impl Cache for FakeCache {
        fn name(&self) -> &'static str {
            "fake-cache"
        }

        async fn get(&self, _key: &str) -> Result<Option<String>, ProviderError> {
            if self.healthy {
                Ok(None)
            } else {
                Err(ProviderError::Unavailable {
                    category: ProviderCategory::Cache,
                    detail: "fake cache: connection refused".to_string(),
                })
            }
        }

        async fn set(&self, _key: &str, _value: &str) -> Result<(), ProviderError> {
            if self.healthy {
                Ok(())
            } else {
                Err(ProviderError::Unavailable {
                    category: ProviderCategory::Cache,
                    detail: "fake cache: connection refused".to_string(),
                })
            }
        }
    }

    pub struct FakeNotifier {
        pub healthy: AtomicBool,
        pub sent: Mutex<Vec<String>>,
    }

    impl FakeNotifier {
        pub fn new(healthy: bool) -> Self {
            Self {
                healthy: AtomicBool::new(healthy),
                sent: Mutex::new(Vec::new()),
            }
        }

        pub fn set_healthy(&self, healthy: bool) {
            self.healthy.store(healthy, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl Notifier for FakeNotifier {
        fn name(&self) -> &'static str {
            "fake-notifier"
        }

        async fn notify(&self, message: &str) -> Result<(), ProviderError> {
            if self.healthy.load(Ordering::SeqCst) {
                self.sent.lock().unwrap().push(message.to_string());
                Ok(())
            } else {
                Err(ProviderError::Unavailable {
                    category: ProviderCategory::Notifier,
                    detail: "fake notifier: connection refused".to_string(),
                })
            }
        }
    }

    pub struct FakeCollector {
        pub healthy: bool,
    }

    #[async_trait]
    impl Collector for FakeCollector {
        fn name(&self) -> &'static str {
            "fake-collector"
        }

        async fn poll(&self) -> Result<CollectorStatus, ProviderError> {
            if self.healthy {
                Ok(CollectorStatus::Healthy)
            } else {
                Err(ProviderError::Unavailable {
                    category: ProviderCategory::Collector,
                    detail: "fake collector: api server unreachable".to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;
    use crate::cache::InProcessCache;

    #[tokio::test]
    async fn store_fails_fast_when_configured_and_unreachable() {
        let store = FakeStore { healthy: false };
        let result = resolve_store(Some(&store)).await;
        assert!(
            result.is_err(),
            "unreachable configured store must not be silently accepted"
        );
    }

    #[tokio::test]
    async fn store_ok_when_nothing_configured() {
        let result = resolve_store(None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn store_ok_when_configured_and_healthy() {
        let store = FakeStore { healthy: true };
        assert!(resolve_store(Some(&store)).await.is_ok());
    }

    #[tokio::test]
    async fn cache_degrades_to_default_on_failure_and_logs_once() {
        let configured: Arc<dyn Cache> = Arc::new(FakeCache { healthy: false });
        let default: Arc<dyn Cache> = Arc::new(InProcessCache::new());
        let cache = DegradingCache::new(Some(configured), default);

        assert!(!cache.is_degraded());
        cache
            .set("k", "v")
            .await
            .expect("degrade path must not surface an error");
        assert!(cache.is_degraded());
        let value = cache
            .get("k")
            .await
            .expect("degraded reads must hit the default");
        assert_eq!(value.as_deref(), Some("v"));
    }

    #[tokio::test]
    async fn cache_uses_configured_when_healthy() {
        let configured: Arc<dyn Cache> = Arc::new(InProcessCache::new());
        let default: Arc<dyn Cache> = Arc::new(FakeCache { healthy: false });
        let cache = DegradingCache::new(Some(configured), default);

        cache.set("k", "v").await.unwrap();
        assert!(!cache.is_degraded());
    }

    #[tokio::test]
    async fn notifier_queues_on_failure_instead_of_erroring() {
        let inner = Arc::new(FakeNotifier::new(false));
        let notifier = RetryingNotifier::new(inner, 2);

        assert!(
            notifier.notify("first").await.is_ok(),
            "notify must never block/error on a probe"
        );
        assert_eq!(notifier.queue_len(), 1);
    }

    #[tokio::test]
    async fn notifier_drops_oldest_when_queue_full() {
        let inner = Arc::new(FakeNotifier::new(false));
        let notifier = RetryingNotifier::new(inner, 2);

        notifier.notify("a").await.unwrap();
        notifier.notify("b").await.unwrap();
        notifier.notify("c").await.unwrap();

        assert_eq!(
            notifier.queue_len(),
            2,
            "queue must stay bounded at capacity"
        );
    }

    #[tokio::test]
    async fn notifier_drains_queue_once_sink_recovers() {
        let inner = Arc::new(FakeNotifier::new(false));
        let notifier = RetryingNotifier::new(Arc::clone(&inner) as Arc<dyn Notifier>, 4);

        notifier.notify("a").await.unwrap();
        notifier.notify("b").await.unwrap();
        assert_eq!(notifier.queue_len(), 2);

        inner.set_healthy(true);
        notifier.drain_retries().await;

        assert_eq!(notifier.queue_len(), 0);
        assert_eq!(inner.sent.lock().unwrap().as_slice(), ["a", "b"]);
    }

    #[tokio::test]
    async fn collector_becomes_unknown_on_failure_not_an_error() {
        let collector = FakeCollector { healthy: false };
        let status = poll_collector_safely(&collector).await;
        assert!(matches!(status, CollectorStatus::Unknown { .. }));
    }

    #[tokio::test]
    async fn collector_reports_healthy_when_reachable() {
        let collector = FakeCollector { healthy: true };
        let status = poll_collector_safely(&collector).await;
        assert_eq!(status, CollectorStatus::Healthy);
    }
}
