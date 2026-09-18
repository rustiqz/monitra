//! `Agent` — a registered remote collector (DESIGN.md §5.1, ADR-008).

use serde::{Deserialize, Serialize};

/// A `monitra agent run` instance watching a host or a Kubernetes cluster it
/// pushes into. Low row count, low write volume (heartbeats, not check
/// data).
///
/// An agent's own liveness is tracked separately from any `Monitor`'s
/// status, for the same reason `Pending` exists (§5.2): "the agent went
/// silent" and "the target is down" are different failure signals and must
/// never be collapsed into one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: u64,
    pub name: String,
    pub last_heartbeat_at: u64,
    pub scope: String,
}
