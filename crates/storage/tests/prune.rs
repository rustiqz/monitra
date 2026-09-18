//! Retention pruning (DESIGN.md §5.4): only raw `check_results` older than
//! the cutoff are removed.

use monitra_models::{CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_provider::Store;
use monitra_storage::SqliteStore;

#[tokio::test]
async fn prune_removes_only_rows_older_than_cutoff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SqliteStore::open(dir.path().join("monitra.db")).expect("open sqlite store");

    let monitor = store
        .insert_monitor(Monitor {
            id: 0,
            name: "example".to_string(),
            target: "https://example.com".to_string(),
            kind: MonitorKind::Http,
            interval_secs: 30,
            status: MonitorStatus::Pending,
        })
        .await
        .expect("insert monitor");

    let make_result = |checked_at: u64| CheckResult {
        monitor_id: monitor.id,
        checked_at,
        success: true,
        latency_ms: 1,
        message: None,
    };
    store
        .insert_check_results(&[make_result(1_000), make_result(2_000), make_result(3_000)])
        .await
        .expect("insert check results");

    let pruned = store
        .prune_check_results_older_than(2_500)
        .await
        .expect("prune");
    assert_eq!(pruned, 2);

    let remaining = store
        .list_check_results(monitor.id, None)
        .await
        .expect("list check results");
    assert_eq!(remaining, vec![make_result(3_000)]);
}
