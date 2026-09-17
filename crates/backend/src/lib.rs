//! API surface — the sole source of truth for clients (DESIGN.md §4
//! `backend`, ADR-009).
//!
//! Owns the Axum router, HTTP handlers, WebSocket fan-out, the authenticated
//! agent-ingest endpoint (ADR-008), human-facing API auth (ADR-009), and
//! embedded web-dashboard asset serving (Phase 10). A thin translation layer
//! — business logic belongs in `engine`, not here.
//!
//! Empty as of Phase 1 (scaffolding). Router and handlers land at Phase 5.
