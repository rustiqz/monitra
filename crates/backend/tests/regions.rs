//! Phase 11 gate (ADR-011): `GET /regions` aggregates a target monitored
//! from multiple region-tagged agents correctly, and an agent with no
//! declared region is excluded entirely — never defaulted into some
//! catch-all bucket (§5.1/ADR-011).

mod support;

use std::sync::Arc;

use monitra_engine::AssignmentHandle;
use monitra_models::{Agent, CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_provider::Store;
use serde_json::Value;
use support::InMemoryStore;
use tokio::sync::broadcast;

const TOKEN: &str = "test-token-0123456789";

async fn spawn_server(store: Arc<InMemoryStore>) -> String {
    let (results_tx, _unused_rx) = broadcast::channel(16);
    let (ingest, _unused_ingest_rx) = monitra_engine::IngestHandle::channel(16);
    let (notifier, cache, k8s_clusters) = support::test_backend_extras();

    let app = monitra_backend::router(
        store as Arc<dyn Store>,
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
        results_tx,
        ingest,
        AssignmentHandle::new(16),
        notifier,
        cache,
        k8s_clusters,
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

async fn seed_agent(store: &InMemoryStore, name: &str, region: Option<&str>) -> u64 {
    let agent = store
        .upsert_agent(Agent {
            id: 0,
            name: name.to_string(),
            last_heartbeat_at: 1_000,
            scope: "host:test".to_string(),
            token: format!("token-{name}"),
            region: region.map(str::to_string),
        })
        .await
        .expect("upsert agent");
    agent.id
}

async fn seed_monitor(store: &InMemoryStore, target: &str, agent_id: u64) -> u64 {
    let monitor = store
        .insert_monitor(Monitor {
            id: 0,
            name: format!("{target}@{agent_id}"),
            target: target.to_string(),
            kind: MonitorKind::Http,
            interval_secs: 30,
            status: MonitorStatus::Up,
            agent_id: Some(agent_id),
        })
        .await
        .expect("insert monitor");
    monitor.id
}

async fn seed_results(store: &InMemoryStore, monitor_id: u64, latencies_ms: &[u64]) {
    let results: Vec<CheckResult> = latencies_ms
        .iter()
        .enumerate()
        .map(|(i, latency_ms)| CheckResult {
            monitor_id,
            checked_at: 1_000 + i as u64,
            success: true,
            latency_ms: *latency_ms,
            message: None,
        })
        .collect();
    store
        .insert_check_results(&results)
        .await
        .expect("insert check results");
}

#[tokio::test]
async fn a_target_monitored_from_two_regions_aggregates_separately() {
    let store = Arc::new(InMemoryStore::new());

    let us_agent = seed_agent(&store, "us-agent", Some("us-east")).await;
    let eu_agent = seed_agent(&store, "eu-agent", Some("eu-west")).await;

    let us_monitor = seed_monitor(&store, "https://example.com", us_agent).await;
    let eu_monitor = seed_monitor(&store, "https://example.com", eu_agent).await;

    seed_results(&store, us_monitor, &[10, 20, 30]).await;
    seed_results(&store, eu_monitor, &[100, 200]).await;

    let base = spawn_server(Arc::clone(&store)).await;
    let client = reqwest::Client::new();
    let body: Value = client
        .get(format!("{base}/regions"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("request")
        .json()
        .await
        .expect("json");

    let aggregates = body.as_array().expect("array response");
    assert_eq!(
        aggregates.len(),
        2,
        "expected exactly one aggregate per region, got {aggregates:?}"
    );

    let by_region = |region: &str| {
        aggregates
            .iter()
            .find(|a| a["region"] == region)
            .unwrap_or_else(|| panic!("no aggregate for region {region} in {aggregates:?}"))
    };

    let us = by_region("us-east");
    assert_eq!(us["target"], "https://example.com");
    assert_eq!(us["monitor_count"], 1);
    assert_eq!(us["probe_count"], 3);
    assert_eq!(us["failure_count"], 0);

    let eu = by_region("eu-west");
    assert_eq!(eu["monitor_count"], 1);
    assert_eq!(eu["probe_count"], 2);
}

#[tokio::test]
async fn an_agent_with_no_region_is_excluded_never_defaulted() {
    let store = Arc::new(InMemoryStore::new());

    let regionless_agent = seed_agent(&store, "regionless-agent", None).await;
    let monitor = seed_monitor(&store, "https://example.com", regionless_agent).await;
    seed_results(&store, monitor, &[10]).await;

    let base = spawn_server(Arc::clone(&store)).await;
    let client = reqwest::Client::new();
    let body: Value = client
        .get(format!("{base}/regions"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("request")
        .json()
        .await
        .expect("json");

    let aggregates = body.as_array().expect("array response");
    assert!(
        aggregates.is_empty(),
        "a monitor whose agent has no region must contribute to no aggregate at all, got {aggregates:?}"
    );
}

#[tokio::test]
async fn regions_route_requires_the_human_token() {
    let base = spawn_server(Arc::new(InMemoryStore::new())).await;
    let response = reqwest::get(format!("{base}/regions"))
        .await
        .expect("request");
    assert_eq!(response.status(), 401);
}
