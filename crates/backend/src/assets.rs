//! Embedded web-dashboard static assets (DESIGN.md §8, §4 `backend`, Phase
//! 10). `build.rs` runs the frontend's `npm run build`; `RustEmbed` reads
//! whatever lands in `web/dist` at compile time.
//!
//! Mounted as the router's fallback, after every API route — this only
//! ever sees a request that didn't match `/health`, an authenticated API
//! route, or the agent-ingest route (`lib.rs`), so it can never
//! accidentally shadow one of those (P1: an auth-gated route staying
//! auth-gated is not something asset-serving gets to relax).

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct WebAssets;

/// The SPA is hash-routed (`#fleet`, `#agents`, …, `web/README.md`) — hash
/// fragments never reach the server, so there is no client-side path
/// routing to reconcile here. Any path that matches a built asset
/// (`/assets/index-*.js`, …) is served as that file; anything else
/// (including `/`) serves `index.html`, exactly like every other
/// hash-routed single-page app.
pub async fn fallback(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty()
        && let Some(file) = WebAssets::get(path)
    {
        return serve(path, file.data.into_owned());
    }
    match WebAssets::get("index.html") {
        Some(file) => serve("index.html", file.data.into_owned()),
        None => (
            StatusCode::NOT_FOUND,
            "monitra-backend: web dashboard assets are not embedded — did \
             the frontend build run? (`web/dist` was empty at compile time)",
        )
            .into_response(),
    }
}

fn serve(path: &str, data: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    ([(header::CONTENT_TYPE, mime.as_ref())], data).into_response()
}
