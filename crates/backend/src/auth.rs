//! Bearer-token auth middleware (DESIGN.md §11.11, resolved Phase 5).
//!
//! A single static API token per instance — no per-user identity, no
//! sessions (the target users, §1.4, are solo operators and small teams,
//! not a multi-tenant deployment). Applied to every route except `/health`,
//! which must answer even when the token has been lost (P6).

//! Bearer-token auth middleware (DESIGN.md §11.11, resolved Phase 5;
//! §11.10, resolved Phase 7).
//!
//! Two independent bearer schemes share this module: [`require_token`], the
//! single static human-facing API token (§11.11); and
//! [`require_agent_token`], a per-agent push token (§11.10) scoped to that
//! one agent's own `/agents/{id}/ingest` route — deliberately a *different*
//! credential, not a reuse of the human token, so a leaked agent token only
//! ever exposes that one agent's push path.

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

use crate::AppState;

/// Constant-time byte comparison — an equality check on a token is a timing
/// side-channel otherwise. Hand-rolled rather than a new dependency (e.g.
/// `subtle`) for a single small compare.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn bearer_token(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

pub async fn require_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    match bearer_token(&request) {
        Some(token) if constant_time_eq(token.as_bytes(), state.token.as_bytes()) => {
            Ok(next.run(request).await)
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Checked against the one `Agent` named by the `{id}` path segment, not
/// the human token — an unknown agent id and a wrong token both answer 401
/// (never 404), so an unauthenticated caller can't use this route to probe
/// which agent ids exist.
pub async fn require_agent_token(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(provided) = bearer_token(&request) else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let agent = state
        .store
        .get_agent(id)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    match agent {
        Some(agent) if constant_time_eq(provided.as_bytes(), agent.token.as_bytes()) => {
            Ok(next.run(request).await)
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
