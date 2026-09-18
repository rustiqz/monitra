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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorStatus {
    Pending,
    Up,
    Down,
    Paused,
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
}
