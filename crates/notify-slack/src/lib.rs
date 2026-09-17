//! Optional `Notifier` provider: Slack (DESIGN.md §4.1, ADR-007).
//!
//! Gated behind the `slack` cargo feature (Cargo.toml, root). Queued with
//! bounded retry/backoff like every `Notifier` — never blocks a probe.
//!
//! Empty as of Phase 1 (scaffolding).
