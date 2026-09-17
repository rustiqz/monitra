//! Optional `Store` provider: Postgres (DESIGN.md §4.1, ADR-002/ADR-007).
//!
//! Gated behind the `postgres` cargo feature (Cargo.toml, root). Attached by
//! URL via config/`monitra setup`, never a default — unreachable-when-
//! configured fails the daemon fast rather than falling back to SQLite.
//!
//! Empty as of Phase 1 (scaffolding).
