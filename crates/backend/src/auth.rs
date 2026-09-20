//! Bearer-token auth middleware (DESIGN.md §11.11, resolved Phase 5).
//!
//! A single static API token per instance — no per-user identity, no
//! sessions (the target users, §1.4, are solo operators and small teams,
//! not a multi-tenant deployment). Applied to every route except `/health`,
//! which must answer even when the token has been lost (P6).

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

use crate::AppState;

/// Constant-time byte comparison — an equality check on the token is a
/// timing side-channel otherwise. Hand-rolled rather than a new dependency
/// (e.g. `subtle`) for a single 64-byte compare.
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

pub async fn require_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let provided = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    match provided {
        Some(token) if constant_time_eq(token.as_bytes(), state.token.as_bytes()) => {
            Ok(next.run(request).await)
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
