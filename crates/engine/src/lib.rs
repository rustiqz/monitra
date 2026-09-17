//! The monitoring core (DESIGN.md §4 `engine`).
//!
//! Owns the scheduler, probe execution (incl. Collector-based K8s polling,
//! ADR-008), timeout/retry/backoff, concurrency limiting, status-transition
//! logic (flap damping + agent-liveness watchdog), result broadcast, and
//! `AlertEvent` emission. The crate where P1 (reliability) matters most.
//!
//! Empty as of Phase 1 (scaffolding). Scheduler and probes land at Phase 6.
