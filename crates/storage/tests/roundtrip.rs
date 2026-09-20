//! Insert-then-read round-trips for all four §5.1 entities, and the
//! monitor-delete cascade, against a fresh on-disk database.

use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_provider::Store;
use monitra_storage::SqliteStore;

fn open_temp() -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SqliteStore::open(dir.path().join("monitra.db")).expect("open sqlite store");
    (dir, store)
}

#[tokio::test]
async fn monitor_round_trips() {
    let (_dir, store) = open_temp();

    let inserted = store
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
    assert_ne!(inserted.id, 0);

    let fetched = store
        .get_monitor(inserted.id)
        .await
        .expect("get monitor")
        .expect("monitor exists");
    assert_eq!(fetched, inserted);

    let listed = store.list_monitors().await.expect("list monitors");
    assert_eq!(listed, vec![inserted.clone()]);

    store
        .update_monitor(
            inserted.id,
            Some("renamed".to_string()),
            None,
            Some(60),
            None,
        )
        .await
        .expect("update monitor");
    let updated = store
        .get_monitor(inserted.id)
        .await
        .expect("get monitor")
        .expect("monitor exists");
    assert_eq!(updated.name, "renamed");
    assert_eq!(updated.target, inserted.target);
    assert_eq!(updated.interval_secs, 60);

    store
        .set_monitor_status(inserted.id, MonitorStatus::Up)
        .await
        .expect("set status");
    let after_status = store
        .get_monitor(inserted.id)
        .await
        .expect("get monitor")
        .expect("monitor exists");
    assert_eq!(after_status.status, MonitorStatus::Up);
}

#[tokio::test]
async fn check_result_round_trips() {
    let (_dir, store) = open_temp();
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

    let results = vec![
        CheckResult {
            monitor_id: monitor.id,
            checked_at: 1_000,
            success: true,
            latency_ms: 42,
            message: None,
        },
        CheckResult {
            monitor_id: monitor.id,
            checked_at: 2_000,
            success: false,
            latency_ms: 7,
            message: Some("connection refused".to_string()),
        },
    ];
    store
        .insert_check_results(&results)
        .await
        .expect("insert check results");

    let listed = store
        .list_check_results(monitor.id, None)
        .await
        .expect("list check results");
    assert_eq!(listed, results);

    let since = store
        .list_check_results(monitor.id, Some(1_500))
        .await
        .expect("list check results since");
    assert_eq!(since, vec![results[1].clone()]);
}

#[tokio::test]
async fn agent_round_trips() {
    let (_dir, store) = open_temp();

    let inserted = store
        .upsert_agent(Agent {
            id: 0,
            name: "host-a".to_string(),
            last_heartbeat_at: 1_000,
            scope: "host:web-1".to_string(),
        })
        .await
        .expect("upsert agent");
    assert_ne!(inserted.id, 0);

    // Second upsert with the same name updates in place rather than
    // creating a second row.
    let updated = store
        .upsert_agent(Agent {
            id: 0,
            name: "host-a".to_string(),
            last_heartbeat_at: 2_000,
            scope: "host:web-1".to_string(),
        })
        .await
        .expect("upsert agent again");
    assert_eq!(updated.id, inserted.id);
    assert_eq!(updated.last_heartbeat_at, 2_000);

    store
        .heartbeat_agent(inserted.id, 3_000)
        .await
        .expect("heartbeat agent");
    let fetched = store
        .get_agent(inserted.id)
        .await
        .expect("get agent")
        .expect("agent exists");
    assert_eq!(fetched.last_heartbeat_at, 3_000);

    let listed = store.list_agents().await.expect("list agents");
    assert_eq!(listed, vec![fetched]);
}

#[tokio::test]
async fn alert_event_round_trips() {
    let (_dir, store) = open_temp();
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

    let inserted = store
        .insert_alert_event(AlertEvent {
            id: 0,
            monitor_id: monitor.id,
            transitioned_to: MonitorStatus::Down,
            occurred_at: 5_000,
            sinks_attempted: "[\"webhook\"]".to_string(),
            delivery_outcome: "webhook: delivered".to_string(),
        })
        .await
        .expect("insert alert event");
    assert_ne!(inserted.id, 0);

    let listed = store
        .list_alert_events(monitor.id)
        .await
        .expect("list alert events");
    assert_eq!(listed, vec![inserted]);
}

#[tokio::test]
async fn deleting_a_monitor_cascades_to_its_history() {
    let (_dir, store) = open_temp();
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
    store
        .insert_check_results(&[CheckResult {
            monitor_id: monitor.id,
            checked_at: 1_000,
            success: true,
            latency_ms: 5,
            message: None,
        }])
        .await
        .expect("insert check result");
    store
        .insert_alert_event(AlertEvent {
            id: 0,
            monitor_id: monitor.id,
            transitioned_to: MonitorStatus::Up,
            occurred_at: 1_000,
            sinks_attempted: "[]".to_string(),
            delivery_outcome: "".to_string(),
        })
        .await
        .expect("insert alert event");

    store
        .delete_monitor(monitor.id)
        .await
        .expect("delete monitor");

    assert_eq!(
        store
            .list_check_results(monitor.id, None)
            .await
            .expect("list check results"),
        Vec::new()
    );
    assert_eq!(
        store
            .list_alert_events(monitor.id)
            .await
            .expect("list alert events"),
        Vec::new()
    );
}
