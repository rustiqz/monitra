//! Local host checks (DESIGN.md §4 `agent`, ADR-008) — systemd unit status,
//! disk space, process liveness. No black-box network equivalent exists for
//! any of these; that is the entire reason an agent, not just a prober,
//! exists (ADR-008).

mod disk;
mod process;
mod systemd;

pub use disk::check_disk;
pub use process::check_process;
pub use systemd::check_systemd;

use crate::config::CheckDefinition;

/// Mirrors `monitra_engine::ProbeOutcome`'s three-way split, kept as its own
/// type here — `monitra-agent` may never depend on `monitra-engine` or
/// `monitra-provider` (DAG, ADR-008). `client.rs` is what translates this
/// into the wire DTO.
#[derive(Debug, Clone, PartialEq)]
pub enum CheckOutcome {
    Success {
        latency_ms: u64,
    },
    Failure {
        message: String,
    },
    /// The check itself could not run (permission denied, missing tooling,
    /// unreadable `/proc`) — distinct from a target that was checked and
    /// found down (P1, §11.3's honesty requirement applied to local checks).
    Unavailable {
        message: String,
    },
}

/// Dispatches one configured check to its implementation.
pub async fn run_check(definition: &CheckDefinition) -> CheckOutcome {
    match definition {
        CheckDefinition::Disk {
            path, min_free_pct, ..
        } => check_disk(path, *min_free_pct),
        CheckDefinition::Systemd { unit, .. } => check_systemd(unit).await,
        CheckDefinition::Process { name, .. } => check_process(name),
    }
}
