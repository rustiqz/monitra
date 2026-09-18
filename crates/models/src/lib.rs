//! Shared domain vocabulary (DESIGN.md §4 `models`).
//!
//! Zero internal dependencies — every other crate may depend on this one,
//! this one depends on nothing internal. Contains only plain data types and
//! pure functions over them; no persistence, no I/O, no HTTP representations,
//! no argument-parsing concerns (that's `cli`'s job — see `MonitorKindArg`
//! there and its `From` conversion into `MonitorKind`).

mod agent;
mod alert_event;
mod check_result;
mod monitor;

pub use agent::Agent;
pub use alert_event::AlertEvent;
pub use check_result::CheckResult;
pub use monitor::{Monitor, MonitorKind, MonitorStatus};
