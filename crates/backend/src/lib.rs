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
mod auth;
mod error;
mod health;
mod ingest;
mod monitors;
pub mod service;
mod ws;

pub use error::BackendError;

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};
use monitra_engine::IngestHandle;
use monitra_models::CheckResult;
use monitra_provider::Store;
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
}

/// Builds the full router: `/health` unauthenticated; `/agents/{id}/ingest`
/// gated behind that one agent's own push token (§11.10); everything else
/// gated behind the human bearer token (§11.11).
pub fn router(
    store: Arc<dyn Store>,
    token: String,
    version: String,
    results: broadcast::Sender<CheckResult>,
    ingest: IngestHandle,
) -> Router {
    let state = AppState {
        store,
        token: Arc::from(token),
        version: Arc::from(version),
        results,
        ingest,
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
        .route("/agents", post(agents::register).get(agents::list))
        .route("/agents/{id}", delete(agents::remove))
        .route("/ws", get(ws::upgrade))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

    let agent_ingest = Router::new()
        .route("/agents/{id}/ingest", post(ingest::push))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_agent_token,
        ));

    Router::new()
        .route("/health", get(health::health))
        .merge(authenticated)
        .merge(agent_ingest)
        .with_state(state)
}

/// Binds `bind_addr` and serves `router` until the process is killed. The
/// one place `main.rs` needs to reach for this crate's Axum/tokio-net
/// details — `Start`'s CLI execution never touches `axum` directly.
pub async fn serve(bind_addr: &str, router: Router) -> Result<(), BackendError> {
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|source| BackendError::Bind {
            addr: bind_addr.to_string(),
            source,
        })?;
    tracing::info!(addr = bind_addr, "backend: listening");
    axum::serve(listener, router)
        .await
        .map_err(BackendError::Serve)
}
