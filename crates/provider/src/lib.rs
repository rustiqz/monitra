//! Pluggable service contracts (DESIGN.md §4 `provider`).
//!
//! Owns the `Store`, `Cache`, `Notifier`, and `Collector` (ADR-008) traits,
//! the provider registry, config parsing/resolution, and the per-category
//! availability rules of §4.1. Depends only on `models`; knows nothing about
//! any concrete implementation.
//!
//! Empty as of Phase 1 (scaffolding). Traits and registry land at Phase 3.
