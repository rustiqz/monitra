//! `GET /health` (DESIGN.md P6) — reports internal state, not just `200 OK`.
//! Unauthenticated: it must answer even if the API token has been lost.
//!
//! A store failure is reported as `store.reachable: false`, not surfaced as
//! an HTTP error and never conflated with a monitor being "down" (P1,
//! §11.3) — the failure is on our side, not the target's.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    status: &'static str,
    version: String,
    store: StoreHealth,
    /// Reported even though nothing calls `Cache::get`/`set` yet (§4
    /// `provider`) — an honest "what's configured," not a fabricated
    /// reachability claim (P1). Read by the TUI/web Services screen.
    cache: CacheHealth,
    notifier: NotifierHealth,
    k8s_clusters: Vec<String>,
}

#[derive(Serialize)]
struct StoreHealth {
    name: &'static str,
    reachable: bool,
}

#[derive(Serialize)]
struct CacheHealth {
    name: &'static str,
    degraded: bool,
}

#[derive(Serialize)]
struct NotifierHealth {
    name: &'static str,
    queue_len: usize,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let reachable = state.store.health_check().await.is_ok();
    Json(HealthResponse {
        status: "ok",
        version: state.version.to_string(),
        store: StoreHealth {
            name: state.store.name(),
            reachable,
        },
        cache: CacheHealth {
            name: state.cache.active_name(),
            degraded: state.cache.is_degraded(),
        },
        notifier: NotifierHealth {
            name: state.notifier.inner_name(),
            queue_len: state.notifier.queue_len(),
        },
        k8s_clusters: state.k8s_clusters.to_vec(),
    })
}
