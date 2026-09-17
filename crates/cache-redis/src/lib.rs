//! Optional `Cache` provider: Redis (DESIGN.md §4.1, ADR-007).
//!
//! Gated behind the `redis` cargo feature (Cargo.toml, root). Unreachable-
//! when-configured degrades to the in-process default and logs at WARN —
//! a cache miss costs latency, nothing more.
//!
//! Empty as of Phase 1 (scaffolding).
