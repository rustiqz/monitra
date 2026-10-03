//! End-to-end startup retention with the real SQLite Store and engine.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use monitra_models::{CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_provider::{RetryingNotifier, Store};

#[tokio::test]
async fn startup_pruning_waits_for_readiness_then_removes_only_expired_results() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store: Arc<dyn Store> = Arc::new(
        monitra_storage::SqliteStore::open(dir.path().join("retention.db")).expect("open store"),
    );
    let monitor = store
        .insert_monitor(Monitor {
            id: 0,
            name: "paused".to_string(),
            target: "http://example.com".to_string(),
            kind: MonitorKind::Http,
            interval_secs: 30,
            status: MonitorStatus::Paused,
            agent_id: None,
        })
        .await
        .expect("insert monitor");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let result = |checked_at| CheckResult {
        monitor_id: monitor.id,
        checked_at,
        success: true,
        latency_ms: 1,
        message: None,
    };
    store
        .insert_check_results(&[result(1), result(now)])
        .await
        .expect("seed results");

    let notifier = Arc::new(RetryingNotifier::new(
        Arc::new(notify_webhook::WebhookNotifier::new(None)),
        8,
    ));
    let engine = monitra_engine::EngineHandle::start(
        monitra_engine::EngineDeps {
            store: Arc::clone(&store),
            k8s_factory: None,
            notifier,
        },
        monitra_engine::EngineConfig::default(),
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        store
            .list_check_results(monitor.id, None)
            .await
            .expect("before ready")
            .len(),
        2
    );
    engine.mark_ready();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if store
                .list_check_results(monitor.id, None)
                .await
                .expect("after ready")
                == vec![result(now)]
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("startup prune did not run");
    engine.shutdown().await;
}
