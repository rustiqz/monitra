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
            agent_id: None,
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
        .prune_check_results_older_than(2_500, 10_000)
        .await
        .expect("prune");
    assert_eq!(pruned, 2);

    let remaining = store
        .list_check_results(monitor.id, None)
        .await
        .expect("list check results");
    assert_eq!(remaining, vec![make_result(3_000)]);
}

#[tokio::test]
async fn prune_respects_batch_limit_and_reports_backlog() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SqliteStore::open(dir.path().join("monitra.db")).expect("open sqlite store");
    let monitor = store
        .insert_monitor(Monitor {
            id: 0,
            name: "batch".to_string(),
            target: "http://example.com".to_string(),
            kind: MonitorKind::Http,
            interval_secs: 30,
            status: MonitorStatus::Pending,
            agent_id: None,
        })
        .await
        .expect("insert monitor");
    let results: Vec<CheckResult> = (0..5)
        .map(|checked_at| CheckResult {
            monitor_id: monitor.id,
            checked_at,
            success: true,
            latency_ms: 1,
            message: None,
        })
        .collect();
    store
        .insert_check_results(&results)
        .await
        .expect("insert results");
    assert_eq!(
        store
            .prune_check_results_older_than(10, 2)
            .await
            .expect("prune"),
        2
    );
    assert_eq!(
        store
            .count_prunable_check_results(10, 2)
            .await
            .expect("count"),
        2
    );
    assert_eq!(
        store
            .prune_check_results_older_than(10, 2)
            .await
            .expect("prune"),
        2
    );
    assert_eq!(
        store
            .count_prunable_check_results(10, 2)
            .await
            .expect("count"),
        1
    );
}
