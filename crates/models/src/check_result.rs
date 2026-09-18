//! `CheckResult` — one observation of a `Monitor` (DESIGN.md §5.1).

use serde::{Deserialize, Serialize};

/// High write volume, unbounded growth without a retention policy (§5.4).
/// Populated by both pull (engine's own probes) and push (agent-relayed)
/// paths through the same writer path (§6.2) — this type does not
/// distinguish origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub monitor_id: u64,
    pub checked_at: u64,
    pub success: bool,
    pub latency_ms: u64,
    pub message: Option<String>,
}
