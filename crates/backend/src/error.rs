//! Maps `ProviderError` (and the plain "not found" case Axum's `Option`
//! extraction hits) to HTTP status codes — the one place that translation
//! happens, so no handler repeats it (DESIGN.md §4 `backend`: "a thin
//! translation layer").

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use monitra_provider::ProviderError;
use serde::Serialize;

/// Errors from booting the server itself, as opposed to `ApiError`'s
/// per-request mapping (CLAUDE.md "errors name their component").
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("backend: failed to bind {addr}: {source}")]
    Bind {
        addr: String,
        #[source]
        source: std::io::Error,
    },

    #[error("backend: server error: {0}")]
    Serve(#[source] std::io::Error),
}

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    Provider(ProviderError),
}

impl From<ProviderError> for ApiError {
    fn from(error: ProviderError) -> Self {
        match error {
            ProviderError::NotFound { .. } => ApiError::NotFound,
            other => ApiError::Provider(other),
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::Provider(ProviderError::Unavailable { detail, .. }) => {
                (StatusCode::SERVICE_UNAVAILABLE, detail)
            }
            ApiError::Provider(other) => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}
