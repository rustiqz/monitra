//! Phase 8 gate (ADR-008): `monitra-agent`'s push loop against a real
//! backend — proves delivery lands in the engine's ingest path, and that an
//! unreachable backend never crashes the agent or blocks its local checks
//! (§7.3). Lives at the workspace root, not in `crates/agent/tests/`,
//! because it needs `monitra-agent` wired against a real
//! `monitra-backend` router and a real `SqliteStore` end to end —
//! `monitra-agent` itself is (correctly) forbidden by
//! `scripts/dep-check.py` from depending on either (DAG, ADR-008). Only the
//! root binary is allowed to know about all three.
//!
//! Also covers the Phase 11 gate (ADR-011): a real regional assignment
//! pulled from `GET /agents/{id}/assignments`, probed by the agent's own
//! `monitra_probe::Probers` instance against a real local listener, and
//! pushed back — proving `monitra-probe` produces the same `ProbeOutcome`
//! shape whether it runs in `monitra-engine`'s pull path (`hard_timeout.rs`,
//! now in `crates/probe/tests/`) or here, in `monitra-agent`'s own vantage
//! point.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use monitra_agent::RunConfig;
use monitra_backend::service;
use monitra_engine::{Assignment, AssignmentHandle, IngestHandle, PushedResult};
use monitra_models::MonitorKind;
use monitra_provider::{DegradingCache, InProcessCache, Notifier, ProviderError, Store};
use monitra_storage::SqliteStore;
use tokio::sync::{broadcast, mpsc};

const TOKEN: &str = "human-token-unused-by-this-test";

/// This test only exercises the agent's push loop against a real store, not
/// notification delivery — a no-op sink is enough to satisfy `router()`.
struct NoopNotifier;

#[async_trait]
impl Notifier for NoopNotifier {
    fn name(&self) -> &'static str {
        "noop"
    }

    async fn notify(&self, _message: &str) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// Registers a real agent + a `HostAgentCheck` monitor against a real
/// `SqliteStore`, then boots a real `monitra-backend` router on an
/// ephemeral loopback port. Returns the base URL, the agent's push token,
/// the two ids, the raw ingest channel so the test can observe what the
/// engine would have received without needing a full `EngineHandle`
/// (`crates/backend/tests/events.rs` tests the same way), and the
/// `AssignmentHandle` so a regional-probe test can seed it directly the
/// same way it bypasses a real scheduler for `ingest`.
async fn spawn_backend(
    db_path: &std::path::Path,
) -> (
    String,
    String,
    u64,
    u64,
    mpsc::Receiver<PushedResult>,
    AssignmentHandle,
) {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(db_path).expect("open sqlite store"));

    let agent = service::register_agent(
        store.as_ref(),
        "edge-1".to_string(),
        "host-1".to_string(),
        None,
    )
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
    let assignments = AssignmentHandle::new(16);

    let app = monitra_backend::router(
        Arc::clone(&store),
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
        results_tx,
        ingest,
        assignments.clone(),
        Arc::new(monitra_provider::RetryingNotifier::new(
            Arc::new(NoopNotifier),
            16,
        )),
        Arc::new(DegradingCache::new(None, Arc::new(InProcessCache::new()))),
        Vec::new(),
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
        assignments,
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
    let (base_url, token, agent_id, monitor_id, mut ingest_rx, _assignments) =
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
        probe_timeout: Duration::from_secs(5),
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
        probe_timeout: Duration::from_secs(5),
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

/// Phase 11 gate (ADR-011): a network-probe monitor linked to this agent is
/// enqueued directly on the `AssignmentHandle` (standing in for the
/// scheduler deciding it's due — `crates/engine/src/scheduler.rs`'s own
/// unit tests already cover that decision), pulled via
/// `GET /agents/{id}/assignments`, probed against a real local TCP
/// listener with this agent's own `monitra_probe::Probers`, and pushed back
/// through the same ingest path a local check result already uses.
#[tokio::test]
async fn regional_assignment_is_pulled_probed_and_pushed_back() {
    let db_dir = tempfile::tempdir().expect("tempdir");
    let (base_url, token, agent_id, _host_monitor_id, mut ingest_rx, assignments) =
        spawn_backend(&db_dir.path().join("monitra.db")).await;

    // A real listener the agent's Tcp prober can actually connect to —
    // proves this runs a genuine network probe, not a stub.
    let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind target listener");
    let target_addr = target_listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        // Accept and immediately drop connections for the test's lifetime —
        // enough for a Tcp probe's connect-only check to see success.
        loop {
            if target_listener.accept().await.is_err() {
                break;
            }
        }
    });

    let store: Arc<dyn Store> =
        Arc::new(SqliteStore::open(db_dir.path().join("monitra.db")).expect("reopen store"));
    let regional_monitor = service::add_monitor(
        store.as_ref(),
        "regional-tcp".to_string(),
        target_addr.to_string(),
        MonitorKind::Tcp,
        30,
        Some(agent_id),
    )
    .await
    .expect("add regional monitor");

    assignments.enqueue(
        agent_id,
        Assignment {
            monitor_id: regional_monitor.id,
            target: regional_monitor.target.clone(),
            kind: MonitorKind::Tcp,
        },
    );

    // No local `--config` at all — this agent only has regional work this
    // cycle, proving the pull path stands on its own.
    let handle = tokio::spawn(monitra_agent::run(RunConfig {
        name: "edge-1".to_string(),
        scope: "host-1".to_string(),
        backend_url: base_url,
        agent_id,
        token: Some(token),
        token_file: None,
        config_path: None,
        buffer_capacity: 16,
        probe_timeout: Duration::from_secs(5),
    }));

    let forwarded = tokio::time::timeout(Duration::from_secs(5), ingest_rx.recv())
        .await
        .expect("a result arrived before timing out")
        .expect("channel not closed");
    assert_eq!(forwarded.monitor_id, regional_monitor.id);
    assert!(
        matches!(
            forwarded.outcome,
            monitra_engine::ProbeOutcome::Success { .. }
        ),
        "expected a real Tcp probe success against the local listener, got {:?}",
        forwarded.outcome
    );

    handle.abort();
}
