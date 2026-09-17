//! Remote collection and local host checks (DESIGN.md §4 `agent`, ADR-008).
//!
//! Owns the `monitra agent run` mode: local host checks (systemd/disk/
//! process — no black-box equivalent exists), the Kubernetes-fallback push
//! path, the push loop (retry/backoff, bounded local buffer), and push-
//! authentication. Reports; does not interpret — `engine` decides what a
//! pushed result means once it lands via `backend`'s ingest endpoint.
//!
//! Empty as of Phase 1 (scaffolding). Checks and push loop land at Phase 8.
