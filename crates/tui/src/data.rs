//! The background data layer: polls `monitra-backend`'s REST API on a fixed
//! cadence, refreshed early by `/ws` pushes, and publishes a
//! [`DashboardSnapshot`] the render loop reads. A `tokio::sync::watch`
//! channel is the transport — bounded by construction (always just the
//! latest value, CLAUDE.md's "bounded everything"), which is exactly the
//! semantics a render loop wants: it only ever cares about the current
//! state, never a backlog of past ones.

use std::time::Duration;

use futures_util::StreamExt;
use monitra_models::{Agent, Monitor};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

use crate::client::{AlertEventDto, CheckResultDto, Client, HealthResponse};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const WS_RETRY_DELAY: Duration = Duration::from_secs(3);

#[derive(Clone, Default)]
pub struct DashboardSnapshot {
    pub monitors: Vec<Monitor>,
    pub agents: Vec<Agent>,
    pub alerts: Vec<AlertEventDto>,
    pub health: Option<HealthResponse>,
    /// History for whichever monitor `App` last asked for via the
    /// `selected` channel — `None` until the first fetch lands, or if the
    /// selected monitor has no history yet (`Pending`, never checked).
    pub selected_history: Option<(u64, Vec<CheckResultDto>)>,
    /// Set when the most recent poll failed — the render loop shows this
    /// rather than silently keeping stale data on screen looking fresh
    /// (P1: never let a display imply confidence it doesn't have).
    pub last_error: Option<String>,
}

/// Spawns the poll/WS task and returns the two ends `App` needs: a sender
/// to announce which monitor's history it currently wants, and a receiver
/// for the latest snapshot.
pub fn spawn(
    client: Client,
) -> (
    watch::Sender<Option<u64>>,
    watch::Receiver<DashboardSnapshot>,
) {
    let (selected_tx, selected_rx) = watch::channel(None);
    let (snapshot_tx, snapshot_rx) = watch::channel(DashboardSnapshot::default());
    tokio::spawn(poll_loop(client, selected_rx, snapshot_tx));
    (selected_tx, snapshot_rx)
}

async fn poll_loop(
    client: Client,
    selected_rx: watch::Receiver<Option<u64>>,
    snapshot_tx: watch::Sender<DashboardSnapshot>,
) {
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    let mut ws = WsFeed::connect(client.ws_url()).await;

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            () = ws.wait_for_event() => {}
        }

        let selected = *selected_rx.borrow();
        let snapshot = fetch_snapshot(&client, selected).await;
        // A `Receiver` still exists as long as `App` (and thus this whole
        // process) is alive; a send error only means every receiver was
        // dropped, which happens right as the TUI is shutting down — not
        // an error worth logging on the way out.
        let _ = snapshot_tx.send(snapshot);
    }
}

async fn fetch_snapshot(client: &Client, selected: Option<u64>) -> DashboardSnapshot {
    let (monitors, agents, alerts, health) = tokio::join!(
        client.list_monitors(),
        client.list_agents(),
        client.list_alerts(),
        client.health(),
    );

    let mut last_error = None;
    let mut note_error = |context: &str, error: &dyn std::fmt::Display| {
        last_error = Some(format!("{context}: {error}"));
    };

    let monitors = monitors
        .inspect_err(|e| note_error("monitors", e))
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect();
    let agents = agents
        .inspect_err(|e| note_error("agents", e))
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect();
    let alerts = alerts
        .inspect_err(|e| note_error("alerts", e))
        .unwrap_or_default();
    let health = health.inspect_err(|e| note_error("health", e)).ok();

    let selected_history = match selected {
        Some(id) => match client.monitor_history(id).await {
            Ok(history) => Some((id, history)),
            Err(error) => {
                note_error("history", &error);
                None
            }
        },
        None => None,
    };

    DashboardSnapshot {
        monitors,
        agents,
        alerts,
        health,
        selected_history,
        last_error,
    }
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Best-effort `/ws` subscriber: reconnects on any disconnect after a fixed
/// delay, never panics, and treats "connection down" the same as "no event
/// yet" — the poll interval is the fallback that keeps data moving even if
/// `/ws` never comes back (P1: a missing live feed degrades freshness, it
/// never blocks the dashboard).
struct WsFeed {
    url: String,
    stream: Option<WsStream>,
}

impl WsFeed {
    async fn connect(url: String) -> Self {
        let stream = match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((stream, _response)) => Some(stream),
            Err(error) => {
                tracing::warn!(%error, "tui: /ws connect failed, will retry");
                None
            }
        };
        Self { url, stream }
    }

    /// Resolves once there's something worth re-polling for: a message
    /// arrived, the socket closed, or (while disconnected) the retry delay
    /// elapsed. Never yields a decoded `CheckResult` itself — the snapshot
    /// refetch that follows is the source of truth, this is just the
    /// "wake up early" signal.
    async fn wait_for_event(&mut self) {
        match &mut self.stream {
            Some(stream) => match stream.next().await {
                Some(Ok(Message::Close(_))) | None => {
                    tracing::warn!("tui: /ws closed, reconnecting");
                    self.stream = None;
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    tracing::warn!(%error, "tui: /ws read failed, reconnecting");
                    self.stream = None;
                }
            },
            None => {
                tokio::time::sleep(WS_RETRY_DELAY).await;
                if let Ok((stream, _response)) =
                    tokio_tungstenite::connect_async(self.url.as_str()).await
                {
                    self.stream = Some(stream);
                }
            }
        }
    }
}
