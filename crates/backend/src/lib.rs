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
//! WebSocket fan-out, notifier sinks, and the agent-ingest endpoint remain
//! Phase 7/8.

mod agents;
mod auth;
mod error;
mod health;
mod monitors;
pub mod service;

pub use error::BackendError;

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};
use monitra_provider::Store;

/// Shared handler state. Cheap to clone (`Arc`/`Arc<str>`) — Axum clones it
/// per request. Fields are crate-private; every module here is a child of
/// this one and can reach them directly.
#[derive(Clone)]
pub struct AppState {
    store: Arc<dyn Store>,
    token: Arc<str>,
    version: Arc<str>,
}

/// Builds the full router: `/health` unauthenticated, everything else
/// gated behind the bearer token (§11.11).
pub fn router(store: Arc<dyn Store>, token: String, version: String) -> Router {
    let state = AppState {
        store,
        token: Arc::from(token),
        version: Arc::from(version),
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
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

    Router::new()
        .route("/health", get(health::health))
        .merge(authenticated)
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
