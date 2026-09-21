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

use monitra_probe::ProbeOutcome;

use crate::config::CheckDefinition;

/// Dispatches one configured check to its implementation. Returns
/// `monitra_probe::ProbeOutcome` directly — before ADR-011/Phase 11 this
/// crate had its own `CheckOutcome` duplicating the same three-way split,
/// forced by a DAG position that forbade depending on `monitra-engine`.
/// ADR-011 adds `monitra-probe` (models-only, so agent can depend on it
/// without violating ADR-008) as the one shared leaf for exactly this kind
/// of value, so the duplicate is gone.
pub async fn run_check(definition: &CheckDefinition) -> ProbeOutcome {
    match definition {
        CheckDefinition::Disk {
            path, min_free_pct, ..
        } => check_disk(path, *min_free_pct),
        CheckDefinition::Systemd { unit, .. } => check_systemd(unit).await,
        CheckDefinition::Process { name, .. } => check_process(name),
    }
}
