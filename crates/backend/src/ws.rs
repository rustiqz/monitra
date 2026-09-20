//! `/ws` — live `CheckResult` fan-out (DESIGN.md §3.3, §7.2, Phase 7).
//!
//! Each connection gets its own subscription to `engine`'s shared broadcast
//! channel. A client that falls behind (tokio's broadcast channel reports
//! `Lagged` once it has to skip messages for a slow receiver) is
//! disconnected rather than resumed with a silent gap — resuming without
//! telling the client data was lost would be a P1 violation, and closing
//! the connection never back-pressures the sender (§7.2: "slow clients are
//! dropped, never back-pressure the engine").

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use monitra_models::CheckResult;
use tokio::sync::broadcast;

use crate::AppState;

pub async fn upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    let rx = state.results.subscribe();
    // Echoes the token back as the accepted subprotocol when the client
    // offered it that way (the web dashboard, `auth::require_token_ws`) —
    // a no-op for clients that authenticated via `Authorization` instead
    // (the TUI), which never offered a subprotocol to match against.
    ws.protocols([state.token.to_string()])
        .on_upgrade(move |socket| handle_socket(socket, rx))
}

async fn handle_socket(mut socket: WebSocket, mut rx: broadcast::Receiver<CheckResult>) {
    loop {
        tokio::select! {
            received = rx.recv() => {
                match received {
                    Ok(result) => {
                        let payload = match serde_json::to_string(&result) {
                            Ok(payload) => payload,
                            Err(source) => {
                                tracing::warn!(error = %source, "backend: /ws: failed to serialize check result");
                                continue;
                            }
                        };
                        if socket.send(Message::Text(payload.into())).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            skipped,
                            "backend: /ws client fell behind, closing rather than resuming with a gap"
                        );
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // engine shut down — nothing more to stream.
                        return;
                    }
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    None | Some(Ok(Message::Close(_))) | Some(Err(_)) => return,
                    // Push-only stream — any other client frame (ping/pong
                    // is handled by axum/tungstenite itself) is ignored.
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}
