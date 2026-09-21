//! API surface — the sole source of truth for clients (DESIGN.md §4
//! `backend`, ADR-009).
//!
//! Owns the Axum router, HTTP handlers, WebSocket fan-out, the authenticated
//! agent-ingest endpoint (ADR-008), human-facing API auth (ADR-009), and
//! embedded web-dashboard asset serving (Phase 10). A thin translation layer
//! — business logic belongs in `engine`, not here.
//!
//! Phase 5: router, monitor/agent CRUD, `/health`, bearer-token auth, and
//! the `service` module both this crate's handlers and `main.rs`'s CLI
//! execution call (ADR-009 — "a mutation is never implemented twice").
//! Phase 7: `/ws` live fan-out and the authenticated agent-ingest endpoint.

mod agents;
mod alerts;
mod assets;
mod assignments;
mod auth;
mod error;
mod health;
mod ingest;
mod monitors;
mod regions;
pub mod service;
mod ws;

pub use error::BackendError;

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};
use monitra_engine::{AssignmentHandle, IngestHandle};
use monitra_models::CheckResult;
use monitra_provider::{DegradingCache, RetryingNotifier, Store};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// Shared handler state. Cheap to clone (`Arc`/`Arc<str>`/`broadcast::Sender`
/// /`IngestHandle`) — Axum clones it per request. Fields are crate-private;
/// every module here is a child of this one and can reach them directly.
#[derive(Clone)]
pub struct AppState {
    store: Arc<dyn Store>,
    token: Arc<str>,
    version: Arc<str>,
    /// `engine`'s live `CheckResult` broadcast — `/ws` subscribes its own
    /// receiver per connection (a `broadcast::Receiver` isn't `Clone`, so
    /// the state holds the `Sender` and each handler call subscribes fresh).
    results: broadcast::Sender<CheckResult>,
    /// Where `/agents/{id}/ingest` forwards pushed results into `engine`.
    ingest: IngestHandle,
    /// Where `/agents/{id}/assignments` drains pending regional probes from
    /// `engine`'s scheduler (ADR-011, Phase 11).
    assignments: AssignmentHandle,
    /// Held for `/health` reporting only (Phase 9) — no handler sends
    /// through this; nothing in the daemon calls `Notifier::notify` except
    /// `engine`, which holds its own reference.
    notifier: Arc<RetryingNotifier>,
    /// Held for `/health` reporting only (Phase 9) — nothing yet calls
    /// `Cache::get`/`set` anywhere in the daemon (§4 `provider`); this
    /// exists so the Services screen has an honest answer once something
    /// does, rather than a fabricated one now.
    cache: Arc<DegradingCache>,
    /// Configured Kubernetes cluster names only (Phase 9) — per-resource
    /// live status is already on the `K8s*`-kind `Monitor` rows themselves
    /// (`/monitors`), so this is just "what's attached," not a duplicate of
    /// `Collector::poll()`.
    k8s_clusters: Arc<[String]>,
}

/// Builds the full router: `/health` unauthenticated; `/agents/{id}/ingest`
/// gated behind that one agent's own push token (§11.10); everything else
/// gated behind the human bearer token (§11.11).
#[allow(clippy::too_many_arguments)]
pub fn router(
    store: Arc<dyn Store>,
    token: String,
    version: String,
    results: broadcast::Sender<CheckResult>,
    ingest: IngestHandle,
    assignments: AssignmentHandle,
    notifier: Arc<RetryingNotifier>,
    cache: Arc<DegradingCache>,
    k8s_clusters: Vec<String>,
) -> Router {
    let state = AppState {
        store,
        token: Arc::from(token),
        version: Arc::from(version),
        results,
        ingest,
        assignments,
        notifier,
        cache,
        k8s_clusters: Arc::from(k8s_clusters),
    };

    let authenticated = Router::new()
        .route("/monitors", post(monitors::create).get(monitors::list))
        .route(
            "/monitors/{id}",
            get(monitors::show)
                .patch(monitors::edit)
                .delete(monitors::remove),
        )
        .route("/monitors/{id}/pause", post(monitors::pause))
        .route("/monitors/{id}/resume", post(monitors::resume))
        .route("/monitors/{id}/history", get(monitors::history))
        .route("/agents", post(agents::register).get(agents::list))
        .route("/agents/{id}", delete(agents::remove))
        .route("/alerts", get(alerts::list))
        .route("/regions", get(regions::list))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

    // Split from `authenticated`: a browser's native WebSocket API cannot
    // set an `Authorization` header on the upgrade request, so `/ws` alone
    // also accepts the token via `Sec-WebSocket-Protocol` (§11.11
    // addendum, Phase 10) — `require_token_ws`, not `require_token`.
    let live = Router::new().route("/ws", get(ws::upgrade)).route_layer(
        axum::middleware::from_fn_with_state(state.clone(), auth::require_token_ws),
    );

    let agent_ingest = Router::new()
        .route("/agents/{id}/ingest", post(ingest::push))
        .route("/agents/{id}/assignments", get(assignments::list))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_agent_token,
        ));

    Router::new()
        .route("/health", get(health::health))
        .merge(authenticated)
        .merge(live)
        .merge(agent_ingest)
        .fallback(assets::fallback)
        .with_state(state)
}

/// Binds `bind_addr`, returning the listener before anything is served —
/// the caller (e.g. an embedded-mode `monitra tui`, Phase 9) needs the
/// actual bound address, which matters when `bind_addr` ends in `:0`
/// (an ephemeral port chosen by the OS).
pub async fn bind(bind_addr: &str) -> Result<TcpListener, BackendError> {
    tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|source| BackendError::Bind {
            addr: bind_addr.to_string(),
            source,
        })
}

/// Serves `router` on an already-bound `listener` until the process is
/// killed. The one place callers need to reach for this crate's
/// Axum/tokio-net details — CLI execution never touches `axum` directly.
pub async fn serve(listener: TcpListener, router: Router) -> Result<(), BackendError> {
    let addr = listener
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| "?".to_string());
    tracing::info!(addr, "backend: listening");
    axum::serve(listener, router)
        .await
        .map_err(BackendError::Serve)
}
