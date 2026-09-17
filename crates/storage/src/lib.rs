//! Default `Store` provider: SQLite (DESIGN.md §4 `storage`).
//!
//! Owns the SQLite implementation of the `Store` trait — schema, migrations,
//! connection lifecycle, all SQL, retention/pruning. The always-compiled
//! default; nothing outside this crate constructs raw queries.
//!
//! Empty as of Phase 1 (scaffolding). Schema and queries land at Phase 4.
