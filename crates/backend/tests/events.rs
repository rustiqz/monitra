//! Phase 7 gate: `/ws` fan-out (drops a lagging client without
//! back-pressuring the sender) and `/agents/{id}/ingest` (per-agent auth,
//! batch forwarding, bounded-drop-and-log on a full queue).

mod support;

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use monitra_engine::{AssignmentHandle, IngestHandle, ProbeOutcome, PushedResult};
use monitra_models::CheckResult;
use monitra_provider::Store;
use serde_json::json;
use support::InMemoryStore;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

const TOKEN: &str = "test-token-0123456789";

async fn spawn_server(
    store: Arc<InMemoryStore>,
    results_capacity: usize,
    ingest_capacity: usize,
) -> (
    String,
    broadcast::Sender<CheckResult>,
    mpsc::Receiver<PushedResult>,
) {
    let (results_tx, _unused_rx) = broadcast::channel(results_capacity);
    let (ingest, ingest_rx) = IngestHandle::channel(ingest_capacity);

    let (notifier, cache, k8s_clusters) = support::test_backend_extras();
    let app = monitra_backend::router(
        store as Arc<dyn Store>,
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
        results_tx.clone(),
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

    (format!("http://{addr}"), results_tx, ingest_rx)
}

fn ws_request(
    base: &str,
    token: Option<&str>,
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let url = format!("{}/ws", base.replacen("http://", "ws://", 1));
    let mut request = url.into_client_request().expect("build ws request");
    if let Some(token) = token {
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {token}").parse().expect("header value"),
        );
    }
    request
}

/// A browser can't set `Authorization` on a WS upgrade — it offers the
/// token as a `Sec-WebSocket-Protocol` instead (`auth::require_token_ws`,
/// Phase 10). This builds that request shape directly rather than via
/// `tungstenite`'s subprotocol helper, so the test exercises exactly the
/// header a browser's `new WebSocket(url, [token])` sends.
fn ws_request_via_protocol(
    base: &str,
    token: &str,
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let url = format!("{}/ws", base.replacen("http://", "ws://", 1));
    let mut request = url.into_client_request().expect("build ws request");
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        token.parse().expect("header value"),
    );
    request
}

#[tokio::test]
async fn ws_rejects_connections_without_the_human_token() {
    let (base, _results, _ingest_rx) = spawn_server(Arc::new(InMemoryStore::new()), 16, 16).await;

    let result = tokio_tungstenite::connect_async(ws_request(&base, None)).await;
    assert!(result.is_err(), "expected the handshake to be rejected");
}

#[tokio::test]
async fn ws_accepts_the_token_via_sec_websocket_protocol_and_echoes_it_back() {
    let (base, _results, _ingest_rx) = spawn_server(Arc::new(InMemoryStore::new()), 16, 16).await;

    let (_socket, response) =
        tokio_tungstenite::connect_async(ws_request_via_protocol(&base, TOKEN))
            .await
            .expect("handshake succeeds with the token offered as a subprotocol");

    assert_eq!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .expect("server echoes the accepted subprotocol")
            .to_str()
            .expect("ascii header"),
        TOKEN,
        "server must echo back the offered subprotocol per RFC 6455"
    );
}

#[tokio::test]
async fn ws_rejects_the_wrong_token_offered_via_sec_websocket_protocol() {
    let (base, _results, _ingest_rx) = spawn_server(Arc::new(InMemoryStore::new()), 16, 16).await;

    let result =
        tokio_tungstenite::connect_async(ws_request_via_protocol(&base, "not-the-token")).await;
    assert!(result.is_err(), "expected the handshake to be rejected");
}

#[tokio::test]
async fn ws_streams_broadcast_check_results_to_an_authenticated_client() {
    let (base, results, _ingest_rx) = spawn_server(Arc::new(InMemoryStore::new()), 16, 16).await;

    let (mut socket, _response) = tokio_tungstenite::connect_async(ws_request(&base, Some(TOKEN)))
        .await
        .expect("handshake succeeds with a valid token");

    // Give the server a beat to complete `on_upgrade` and subscribe before
    // publishing — otherwise the send could race the subscribe.
    tokio::time::sleep(Duration::from_millis(20)).await;

    let sent = CheckResult {
        monitor_id: 1,
        checked_at: 1_000,
        success: true,
        latency_ms: 12,
        message: None,
    };
    results.send(sent.clone()).expect("at least one subscriber");

    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("received a message before timing out")
        .expect("stream not closed")
        .expect("no transport error");
    let WsMessage::Text(text) = message else {
        panic!("expected a text frame, got {message:?}");
    };
    let received: CheckResult = serde_json::from_str(&text).expect("valid json check result");
    assert_eq!(received, sent);
}

#[tokio::test]
async fn ws_disconnects_a_client_that_falls_behind_instead_of_resuming_with_a_gap() {
    // A tiny broadcast capacity so a handful of unread sends overflow it.
    let (base, results, _ingest_rx) = spawn_server(Arc::new(InMemoryStore::new()), 2, 16).await;

    let (mut socket, _response) = tokio_tungstenite::connect_async(ws_request(&base, Some(TOKEN)))
        .await
        .expect("handshake succeeds");
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Publish far more than the channel can hold before ever reading —
    // the connection's receiver falls behind and should be dropped, not
    // silently resumed from wherever it can still reach.
    for i in 0..20u64 {
        let _ = results.send(CheckResult {
            monitor_id: i,
            checked_at: i,
            success: true,
            latency_ms: 1,
            message: None,
        });
    }

    // Drain whatever the server managed to forward before it noticed the
    // lag; the stream must end (close/None) rather than deliver all 20
    // messages in order forever.
    let outcome = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Close(_))) | None => return,
                Some(Ok(_)) => continue,
                Some(Err(_)) => return,
            }
        }
    })
    .await;
    assert!(
        outcome.is_ok(),
        "expected the connection to close after falling behind, not hang forever"
    );
}

/// Registers an agent over HTTP (the real flow) and returns its id + token.
async fn register_agent(client: &reqwest::Client, base: &str, name: &str) -> (u64, String) {
    let created: serde_json::Value = client
        .post(format!("{base}/agents"))
        .bearer_auth(TOKEN)
        .json(&json!({ "name": name, "scope": "host:test" }))
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

async fn add_monitor_for_agent(client: &reqwest::Client, base: &str, agent_id: u64) -> u64 {
    let created: serde_json::Value = client
        .post(format!("{base}/monitors"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "name": "host-check",
            "target": "edge-1:disk",
            "kind": "HostAgentCheck",
            "interval_secs": 30,
            "agent_id": agent_id,
        }))
        .send()
        .await
        .expect("create monitor request")
        .json()
        .await
        .expect("create monitor json");
    created["id"].as_u64().expect("id")
}

#[tokio::test]
async fn ingest_rejects_missing_wrong_and_another_agents_token() {
    let (base, _results, _ingest_rx) = spawn_server(Arc::new(InMemoryStore::new()), 16, 16).await;
    let client = reqwest::Client::new();

    let (agent_a, token_a) = register_agent(&client, &base, "agent-a").await;
    let (_agent_b, token_b) = register_agent(&client, &base, "agent-b").await;

    let no_header = client
        .post(format!("{base}/agents/{agent_a}/ingest"))
        .json(&json!({ "results": [] }))
        .send()
        .await
        .expect("request");
    assert_eq!(no_header.status(), 401);

    let wrong_token = client
        .post(format!("{base}/agents/{agent_a}/ingest"))
        .bearer_auth("not-a-real-token")
        .json(&json!({ "results": [] }))
        .send()
        .await
        .expect("request");
    assert_eq!(wrong_token.status(), 401);

    // Agent B's own (valid) token must not authenticate agent A's route.
    let another_agents_token = client
        .post(format!("{base}/agents/{agent_a}/ingest"))
        .bearer_auth(&token_b)
        .json(&json!({ "results": [] }))
        .send()
        .await
        .expect("request");
    assert_eq!(another_agents_token.status(), 401);

    let correct = client
        .post(format!("{base}/agents/{agent_a}/ingest"))
        .bearer_auth(&token_a)
        .json(&json!({ "results": [] }))
        .send()
        .await
        .expect("request");
    assert_eq!(correct.status(), 202);
}

#[tokio::test]
async fn ingest_unknown_agent_id_is_401_not_404() {
    let (base, _results, _ingest_rx) = spawn_server(Arc::new(InMemoryStore::new()), 16, 16).await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{base}/agents/999999/ingest"))
        .bearer_auth("anything")
        .json(&json!({ "results": [] }))
        .send()
        .await
        .expect("request");
    assert_eq!(
        response.status(),
        401,
        "an unauthenticated caller must not be able to distinguish a wrong token from an unknown agent"
    );
}

#[tokio::test]
async fn ingest_forwards_own_results_heartbeats_and_drops_mismatched_monitor() {
    let store = Arc::new(InMemoryStore::new());
    let (base, _results, mut ingest_rx) = spawn_server(Arc::clone(&store), 16, 16).await;
    let client = reqwest::Client::new();

    let (agent_id, token) = register_agent(&client, &base, "edge-1").await;
    let monitor_id = add_monitor_for_agent(&client, &base, agent_id).await;

    let response = client
        .post(format!("{base}/agents/{agent_id}/ingest"))
        .bearer_auth(&token)
        .json(&json!({
            "results": [
                { "monitor_id": monitor_id, "outcome": "success", "latency_ms": 4 },
                // Belongs to no monitor at all — must be dropped, not crash the batch.
                { "monitor_id": 999999, "outcome": "failure", "message": "bogus" },
            ]
        }))
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 202);

    let forwarded = ingest_rx
        .try_recv()
        .expect("the valid result was forwarded to the ingest channel");
    assert_eq!(forwarded.monitor_id, monitor_id);
    assert!(
        ingest_rx.try_recv().is_err(),
        "the mismatched/unknown-monitor result must not have been forwarded at all"
    );

    let agent = store
        .get_agent(agent_id)
        .await
        .expect("get agent")
        .expect("agent exists");
    assert!(
        agent.last_heartbeat_at > 0,
        "a push must update the agent's heartbeat"
    );
}

#[tokio::test]
async fn ingest_unavailable_outcome_is_forwarded_distinct_from_failure() {
    // Phase 8/§11.10 addendum: a local check the agent could not itself run
    // (permission denied, systemctl/dbus unreachable) must arrive at the
    // engine as `Unavailable`, not `Failure` — never collapsed into
    // target-down (P1, §11.3).
    let store = Arc::new(InMemoryStore::new());
    let (base, _results, mut ingest_rx) = spawn_server(Arc::clone(&store), 16, 16).await;
    let client = reqwest::Client::new();

    let (agent_id, token) = register_agent(&client, &base, "edge-1").await;
    let monitor_id = add_monitor_for_agent(&client, &base, agent_id).await;

    let response = client
        .post(format!("{base}/agents/{agent_id}/ingest"))
        .bearer_auth(&token)
        .json(&json!({
            "results": [
                { "monitor_id": monitor_id, "outcome": "unavailable", "message": "systemctl: dbus unreachable" },
            ]
        }))
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 202);

    let forwarded = ingest_rx
        .try_recv()
        .expect("the unavailable result was forwarded to the ingest channel");
    assert_eq!(forwarded.monitor_id, monitor_id);
    assert_eq!(
        forwarded.outcome,
        ProbeOutcome::Unavailable {
            message: "systemctl: dbus unreachable".to_string()
        }
    );
}

#[tokio::test]
async fn ingest_queue_drops_and_logs_without_blocking_when_full() {
    // Capacity 1, and nothing ever drains the ingest channel — deterministic
    // overflow, matching the ingest-queue gate assertion (§7.3, §6.2).
    let (base, _results, mut ingest_rx) = spawn_server(Arc::new(InMemoryStore::new()), 16, 1).await;
    let client = reqwest::Client::new();

    let (agent_id, token) = register_agent(&client, &base, "edge-1").await;
    let monitor_a = add_monitor_for_agent(&client, &base, agent_id).await;
    let monitor_b = add_monitor_for_agent(&client, &base, agent_id).await;

    let response = client
        .post(format!("{base}/agents/{agent_id}/ingest"))
        .bearer_auth(&token)
        .json(&json!({
            "results": [
                { "monitor_id": monitor_a, "outcome": "success", "latency_ms": 1 },
                { "monitor_id": monitor_b, "outcome": "success", "latency_ms": 1 },
            ]
        }))
        .send()
        .await
        .expect("request");
    // The handler itself never blocks or fails because the downstream
    // queue is full — it always accepts the batch (§7.3: drop-and-log is
    // silent to the caller, not an error response).
    assert_eq!(response.status(), 202);

    let first = ingest_rx
        .try_recv()
        .expect("first pushed result was queued");
    assert_eq!(first.monitor_id, monitor_a);
    assert!(
        ingest_rx.try_recv().is_err(),
        "second pushed result must have been dropped, not queued behind the first"
    );
}
