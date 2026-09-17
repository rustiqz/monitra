//! Shared domain vocabulary (DESIGN.md §4 `models`).
//!
//! Zero internal dependencies — every other crate may depend on this one,
//! this one depends on nothing internal. Contains only plain data types and
//! pure functions over them; no persistence, no I/O, no HTTP representations.
//!
//! Empty as of Phase 1 (scaffolding). Real types (`Monitor`, `MonitorKind`,
//! `MonitorStatus`, `CheckResult`, `Agent`, `AlertEvent` — DESIGN.md §5.1)
//! land when a consuming phase first needs them.
