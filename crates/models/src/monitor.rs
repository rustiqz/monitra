//! `Monitor` and its supporting enums (DESIGN.md §5.1).

use serde::{Deserialize, Serialize};

/// What kind of thing a `Monitor` checks (DESIGN.md §5.1).
///
/// The `K8s*` and `HostAgentCheck` variants were added by ADR-008; their
/// `target` interpretation (bare network endpoint vs. orchestrator-resource
/// reference vs. agent-relative check name) is resolved by whichever crate
/// consumes it, not by this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorKind {
    Http,
    Tcp,
    Icmp,
    K8sDeployment,
    K8sStatefulSet,
    K8sService,
    HostAgentCheck,
}

/// A monitor's current status (DESIGN.md §5.2, §5.3).
///
/// `Pending` exists so "never checked yet" is never collapsed into "up" or
/// "down" — the dashboard must not lie during the window between monitor
/// creation and first check (P1).
///
/// `Stale` (added at Phase 6) covers the ADR-008 case in §5.1/§5.2: a
/// monitor fed by an unreachable `Agent` or `Collector` is neither
/// confirmed-up nor confirmed-down, the same "last known, staleness
/// unknown" honesty as `Pending` — just triggered by the feed going silent
/// instead of never having checked at all. Never collapsed into `Down`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorStatus {
    Pending,
    Up,
    Down,
    Paused,
    Stale,
}

/// Configuration for one thing being watched (DESIGN.md §5.1).
///
/// As of ADR-008, a `Monitor` may represent an orchestrator resource (a
/// Kubernetes Deployment/StatefulSet/Service) rather than a bare network
/// endpoint — its identity is the resource, not any one pod backing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monitor {
    pub id: u64,
    pub name: String,
    pub target: String,
    pub kind: MonitorKind,
    pub interval_secs: u64,
    pub status: MonitorStatus,
    /// The `Agent` this monitor depends on for its check data (Phase 6,
    /// `HostAgentCheck`/agent-fed `K8s*` monitors). `None` for monitors the
    /// engine probes directly. Drives the agent-liveness watchdog: when the
    /// referenced `Agent`'s heartbeat times out, this monitor (and every
    /// other one referencing it) moves to `Stale`, never `Down` (§5.1).
    pub agent_id: Option<u64>,
}
