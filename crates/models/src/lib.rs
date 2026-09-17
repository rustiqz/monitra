//! Shared domain vocabulary (DESIGN.md §4 `models`).
//!
//! Zero internal dependencies — every other crate may depend on this one,
//! this one depends on nothing internal. Contains only plain data types and
//! pure functions over them; no persistence, no I/O, no HTTP representations,
//! no argument-parsing concerns (that's `cli`'s job — see `MonitorKindArg`
//! there and its `From` conversion into `MonitorKind`).
//!
//! `MonitorStatus`, `CheckResult`, `Agent`, `AlertEvent` (DESIGN.md §5.1) land
//! when a consuming phase first needs them.

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
