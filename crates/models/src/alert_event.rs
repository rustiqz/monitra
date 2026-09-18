//! `AlertEvent` — a persisted record of an emitted alert (DESIGN.md §5.1,
//! ADR-009).

use serde::{Deserialize, Serialize};

use crate::monitor::MonitorStatus;

/// So alert history is queryable by `cli`/`tui`/web rather than existing
/// only as whatever a `Notifier` sink did with it. A deliberate exception to
/// §5.1's "resist adding entities" discipline — without it, an alert-history
/// view is not buildable at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: u64,
    pub monitor_id: u64,
    pub transitioned_to: MonitorStatus,
    pub occurred_at: u64,
    /// Which `Notifier`s were sent this event (serialized list).
    pub sinks_attempted: String,
    /// Per-sink delivery result, for diagnosing a stuck queue (§7.2).
    pub delivery_outcome: String,
}
