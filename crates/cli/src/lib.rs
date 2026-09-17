//! Argument surface (DESIGN.md §4 `cli`).
//!
//! Owns the `clap` command tree and argument-shape validation only.
//! `Cli::parse()` produces a fully-typed command description; `main.rs`
//! decides what to do with it. No command execution, no I/O.
//!
//! Empty as of Phase 1 (scaffolding). Command tree lands at Phase 2.
