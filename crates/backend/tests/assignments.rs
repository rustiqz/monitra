//! Phase 11 gate (ADR-011): `GET /agents/{id}/assignments` — auth is scoped
//! per agent exactly like `/agents/{id}/ingest` (`auth::require_agent_token`,
//! the same middleware), and draining returns what the engine already
//! enqueued without handing it out twice.

mod support;

use std::sync::Arc;

use monitra_engine::{Assignment, AssignmentHandle};
use monitra_models::MonitorKind;
use monitra_provider::Store;
use serde_json::Value;
use support::InMemoryStore;
use tokio::sync::broadcast;

const TOKEN: &str = "test-token-0123456789";

async fn spawn_server(store: Arc<InMemoryStore>, assignments: AssignmentHandle) -> String {
    let (results_tx, _unused_rx) = broadcast::channel(16);
    let (ingest, _unused_ingest_rx) = monitra_engine::IngestHandle::channel(16);
    let (notifier, cache, k8s_clusters) = support::test_backend_extras();

    let app = monitra_backend::router(
        store as Arc<dyn Store>,
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
        results_tx,
        ingest,
        assignments,
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

/// Registers an agent over HTTP (the real flow) and returns its id + token.
async fn register_agent(client: &reqwest::Client, base: &str, name: &str) -> (u64, String) {
    let created: Value = client
        .post(format!("{base}/agents"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({ "name": name, "scope": "host:test" }))
        .send()
        .await
        .expect("register request")
        .json()
        .await
        .expect("register json");
    (
        created["id"].as_u64().expect("id"),
        created["token"].as_str().expect("token").to_string(),
    )
}

#[tokio::test]
async fn assignments_are_scoped_to_the_owning_agent() {
    let assignments = AssignmentHandle::new(16);
    let store = Arc::new(InMemoryStore::new());
    let base = spawn_server(Arc::clone(&store), assignments.clone()).await;
    let client = reqwest::Client::new();

    let (agent_a, token_a) = register_agent(&client, &base, "agent-a").await;
    let (agent_b, token_b) = register_agent(&client, &base, "agent-b").await;

    assignments.enqueue(
        agent_a,
        Assignment {
            monitor_id: 1,
            target: "https://a.example.com".to_string(),
            kind: MonitorKind::Http,
        },
    );
    assignments.enqueue(
        agent_b,
        Assignment {
            monitor_id: 2,
            target: "https://b.example.com".to_string(),
            kind: MonitorKind::Tcp,
        },
    );

    let for_a: Value = client
        .get(format!("{base}/agents/{agent_a}/assignments"))
        .bearer_auth(&token_a)
        .send()
        .await
        .expect("request")
        .json()
        .await
        .expect("json");
    let list_a = for_a["assignments"].as_array().expect("array");
    assert_eq!(list_a.len(), 1);
    assert_eq!(list_a[0]["monitor_id"], 1);
    assert_eq!(list_a[0]["target"], "https://a.example.com");

    // Agent B's own token must not see agent A's queue, and vice versa.
    let wrong_token = client
        .get(format!("{base}/agents/{agent_a}/assignments"))
        .bearer_auth(&token_b)
        .send()
        .await
        .expect("request");
    assert_eq!(wrong_token.status(), 401);
}

#[tokio::test]
async fn draining_an_assignment_does_not_hand_it_out_twice() {
    let assignments = AssignmentHandle::new(16);
    let store = Arc::new(InMemoryStore::new());
    let base = spawn_server(Arc::clone(&store), assignments.clone()).await;
    let client = reqwest::Client::new();

    let (agent_id, token) = register_agent(&client, &base, "edge-1").await;
    assignments.enqueue(
        agent_id,
        Assignment {
            monitor_id: 7,
            target: "https://example.com".to_string(),
            kind: MonitorKind::Http,
        },
    );

    let first: Value = client
        .get(format!("{base}/agents/{agent_id}/assignments"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request")
        .json()
        .await
        .expect("json");
    assert_eq!(first["assignments"].as_array().expect("array").len(), 1);

    let second: Value = client
        .get(format!("{base}/agents/{agent_id}/assignments"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request")
        .json()
        .await
        .expect("json");
    assert!(
        second["assignments"].as_array().expect("array").is_empty(),
        "an already-drained assignment must not be handed out again"
    );
}

#[tokio::test]
async fn no_assignments_queued_is_an_empty_list_not_an_error() {
    let assignments = AssignmentHandle::new(16);
    let store = Arc::new(InMemoryStore::new());
    let base = spawn_server(Arc::clone(&store), assignments).await;
    let client = reqwest::Client::new();

    let (agent_id, token) = register_agent(&client, &base, "edge-1").await;
    let response = client
        .get(format!("{base}/agents/{agent_id}/assignments"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("json");
    assert!(body["assignments"].as_array().expect("array").is_empty());
}
