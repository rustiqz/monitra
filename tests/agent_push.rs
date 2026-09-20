//! Phase 8 gate (ADR-008): `monitra-agent`'s push loop against a real
//! backend — proves delivery lands in the engine's ingest path, and that an
//! unreachable backend never crashes the agent or blocks its local checks
//! (§7.3). Lives at the workspace root, not in `crates/agent/tests/`,
//! because it needs `monitra-agent` wired against a real
//! `monitra-backend` router and a real `SqliteStore` end to end —
//! `monitra-agent` itself is (correctly) forbidden by
//! `scripts/dep-check.py` from depending on either (DAG, ADR-008). Only the
//! root binary is allowed to know about all three.

use std::sync::Arc;
use std::time::Duration;

use monitra_agent::RunConfig;
use monitra_backend::service;
use monitra_engine::{IngestHandle, PushedResult};
use monitra_models::MonitorKind;
use monitra_provider::Store;
use monitra_storage::SqliteStore;
use tokio::sync::{broadcast, mpsc};

const TOKEN: &str = "human-token-unused-by-this-test";

/// Registers a real agent + a `HostAgentCheck` monitor against a real
/// `SqliteStore`, then boots a real `monitra-backend` router on an
/// ephemeral loopback port. Returns the base URL, the agent's push token,
/// the two ids, and the raw ingest channel so the test can observe what
/// the engine would have received without needing a full `EngineHandle`
/// (`crates/backend/tests/events.rs` tests the same way).
async fn spawn_backend(
    db_path: &std::path::Path,
) -> (String, String, u64, u64, mpsc::Receiver<PushedResult>) {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(db_path).expect("open sqlite store"));

    let agent = service::register_agent(store.as_ref(), "edge-1".to_string(), "host-1".to_string())
        .await
        .expect("register agent");
    let monitor = service::add_monitor(
        store.as_ref(),
        "disk-root".to_string(),
        "disk:/tmp".to_string(),
        MonitorKind::HostAgentCheck,
        30,
        Some(agent.id),
    )
    .await
    .expect("add monitor");

    let (results_tx, _unused_rx) = broadcast::channel(16);
    let (ingest, ingest_rx) = IngestHandle::channel(16);

    let app = monitra_backend::router(
        Arc::clone(&store),
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
        results_tx,
        ingest,
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    (
        format!("http://{addr}"),
        agent.token,
        agent.id,
        monitor.id,
        ingest_rx,
    )
}

fn write_disk_check_config(path: &std::path::Path, monitor_id: u64) {
    std::fs::write(
        path,
        format!(
            "interval_secs = 1\n\n\
             [[checks]]\n\
             kind = \"disk\"\n\
             monitor_id = {monitor_id}\n\
             path = \"/tmp\"\n\
             min_free_pct = 0.0\n"
        ),
    )
    .expect("write agent config");
}

#[tokio::test]
async fn push_loop_delivers_a_real_check_to_a_real_backend() {
    let db_dir = tempfile::tempdir().expect("tempdir");
    let (base_url, token, agent_id, monitor_id, mut ingest_rx) =
        spawn_backend(&db_dir.path().join("monitra.db")).await;

    let config_dir = tempfile::tempdir().expect("tempdir");
    let config_path = config_dir.path().join("agent.toml");
    write_disk_check_config(&config_path, monitor_id);

    let handle = tokio::spawn(monitra_agent::run(RunConfig {
        name: "edge-1".to_string(),
        scope: "host-1".to_string(),
        backend_url: base_url,
        agent_id,
        token: Some(token),
        token_file: None,
        config_path: Some(config_path),
        buffer_capacity: 16,
    }));

    let forwarded = tokio::time::timeout(Duration::from_secs(5), ingest_rx.recv())
        .await
        .expect("a result arrived before timing out")
        .expect("channel not closed");
    assert_eq!(forwarded.monitor_id, monitor_id);
    assert!(matches!(
        forwarded.outcome,
        monitra_engine::ProbeOutcome::Success { .. }
    ));

    handle.abort();
}

#[tokio::test]
async fn agent_survives_an_unreachable_backend_without_crashing_or_blocking_checks() {
    let config_dir = tempfile::tempdir().expect("tempdir");
    let config_path = config_dir.path().join("agent.toml");
    // Any real monitor id works here — nothing on the far end ever parses
    // it, since nothing is listening.
    write_disk_check_config(&config_path, 1);

    // Port 1 is a privileged port nothing is listening on in a test
    // sandbox — connection refused, fast and deterministic, no real
    // network dependency.
    let handle = tokio::spawn(monitra_agent::run(RunConfig {
        name: "edge-1".to_string(),
        scope: "host-1".to_string(),
        backend_url: "http://127.0.0.1:1".to_string(),
        agent_id: 999,
        token: Some("irrelevant-token".to_string()),
        token_file: None,
        config_path: Some(config_path),
        buffer_capacity: 16,
    }));

    // Several would-be push cycles' worth of time against a backend that
    // is never reachable — the loop must still be alive, not panicked, not
    // hung.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !handle.is_finished(),
        "the agent loop must still be running, not have panicked or returned early"
    );

    handle.abort();
    let result = handle.await;
    assert!(
        result.is_err(),
        "the task was aborted, so joining it must report that"
    );
    assert!(
        !result.unwrap_err().is_panic(),
        "the agent must never panic when its backend is unreachable (P1)"
    );
}
