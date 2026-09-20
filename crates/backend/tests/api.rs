//! Real ephemeral-port integration tests (Phase 5 gate): auth, health, and
//! the monitor CRUD round-trip, all over actual HTTP against a real
//! (in-memory, see `support`) `Store` implementation.

mod support;

use std::sync::Arc;

use monitra_provider::Store;
use serde_json::json;
use support::InMemoryStore;

const TOKEN: &str = "test-token-0123456789";

async fn spawn_server(store: Arc<InMemoryStore>) -> String {
    let app = monitra_backend::router(
        store as Arc<dyn Store>,
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
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

#[tokio::test]
async fn health_is_reachable_without_a_token() {
    let base = spawn_server(Arc::new(InMemoryStore::new())).await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{base}/health"))
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.expect("json body");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["store"]["reachable"], true);
}

#[tokio::test]
async fn health_reports_unreachable_store_without_erroring() {
    let store = Arc::new(InMemoryStore::new());
    store.set_healthy(false);
    let base = spawn_server(store).await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{base}/health"))
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.expect("json body");
    assert_eq!(body["store"]["reachable"], false);
}

#[tokio::test]
async fn monitors_route_rejects_missing_or_wrong_token() {
    let base = spawn_server(Arc::new(InMemoryStore::new())).await;
    let client = reqwest::Client::new();

    let no_header = client
        .get(format!("{base}/monitors"))
        .send()
        .await
        .expect("request");
    assert_eq!(no_header.status(), 401);

    let wrong_token = client
        .get(format!("{base}/monitors"))
        .bearer_auth("not-the-token")
        .send()
        .await
        .expect("request");
    assert_eq!(wrong_token.status(), 401);

    let correct_token = client
        .get(format!("{base}/monitors"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("request");
    assert_eq!(correct_token.status(), 200);
}

#[tokio::test]
async fn monitor_crud_round_trips_over_http() {
    let base = spawn_server(Arc::new(InMemoryStore::new())).await;
    let client = reqwest::Client::new();

    let created: serde_json::Value = client
        .post(format!("{base}/monitors"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "name": "example",
            "target": "https://example.com",
            "kind": "Http",
            "interval_secs": 30,
        }))
        .send()
        .await
        .expect("create request")
        .json()
        .await
        .expect("create json");
    let id = created["id"].as_u64().expect("id");
    assert_eq!(created["status"], "Pending");

    let listed: serde_json::Value = client
        .get(format!("{base}/monitors"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("list request")
        .json()
        .await
        .expect("list json");
    assert_eq!(listed.as_array().expect("array").len(), 1);

    let edit_response = client
        .patch(format!("{base}/monitors/{id}"))
        .bearer_auth(TOKEN)
        .json(&json!({ "interval_secs": 60 }))
        .send()
        .await
        .expect("edit request");
    assert_eq!(edit_response.status(), 204);

    let fetched: serde_json::Value = client
        .get(format!("{base}/monitors/{id}"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("show request")
        .json()
        .await
        .expect("show json");
    assert_eq!(fetched["interval_secs"], 60);

    let pause_response = client
        .post(format!("{base}/monitors/{id}/pause"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("pause request");
    assert_eq!(pause_response.status(), 204);

    let resume_response = client
        .post(format!("{base}/monitors/{id}/resume"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("resume request");
    assert_eq!(resume_response.status(), 204);

    let delete_response = client
        .delete(format!("{base}/monitors/{id}"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("delete request");
    assert_eq!(delete_response.status(), 204);

    let not_found = client
        .get(format!("{base}/monitors/{id}"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("show-after-delete request");
    assert_eq!(not_found.status(), 404);
}

#[tokio::test]
async fn agent_register_list_remove_round_trips_over_http() {
    let base = spawn_server(Arc::new(InMemoryStore::new())).await;
    let client = reqwest::Client::new();

    let created: serde_json::Value = client
        .post(format!("{base}/agents"))
        .bearer_auth(TOKEN)
        .json(&json!({ "name": "edge-1", "scope": "host:edge-1" }))
        .send()
        .await
        .expect("register request")
        .json()
        .await
        .expect("register json");
    let id = created["id"].as_u64().expect("id");

    let listed: serde_json::Value = client
        .get(format!("{base}/agents"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("list request")
        .json()
        .await
        .expect("list json");
    assert_eq!(listed.as_array().expect("array").len(), 1);

    let delete_response = client
        .delete(format!("{base}/agents/{id}"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("remove request");
    assert_eq!(delete_response.status(), 204);
}
