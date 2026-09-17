//! Terminal dashboard (DESIGN.md §4 `tui`, ADR-009).
//!
//! A pure HTTP/WS API client against `backend`, in every mode — never reads
//! `storage` directly, even locally (an embedded backend on loopback stands
//! in for a remote daemon when none is running, per ADR-009; see §11.4).
//! Owns terminal setup/teardown, event loop, widgets, view state. Must
//! restore the terminal on every exit path, including panics.
//!
//! Empty as of Phase 1 (scaffolding). Event loop and widgets land at Phase 9.
