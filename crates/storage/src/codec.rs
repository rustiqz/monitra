//! Enum <-> `TEXT` conversions and row mapping. SQL never leaks outside this
//! crate (DESIGN.md §4 `storage`); this is the one place that knows the
//! on-disk string spelling of each `models` enum.
//!
//! Stored as `TEXT`, not `INTEGER` — readable in ad-hoc `sqlite3`
//! inspection at 3 a.m. (P6). Round-trip tests guard against a future
//! rename silently breaking old rows.

use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorKind, MonitorStatus};
use rusqlite::Row;

use crate::error::StorageError;

pub fn query_failed(source: rusqlite::Error) -> StorageError {
    StorageError::QueryFailed { source }
}

pub fn monitor_kind_to_str(kind: MonitorKind) -> &'static str {
    match kind {
        MonitorKind::Http => "http",
        MonitorKind::Tcp => "tcp",
        MonitorKind::Icmp => "icmp",
        MonitorKind::K8sDeployment => "k8s_deployment",
        MonitorKind::K8sStatefulSet => "k8s_stateful_set",
        MonitorKind::K8sService => "k8s_service",
        MonitorKind::HostAgentCheck => "host_agent_check",
    }
}

pub fn monitor_kind_from_str(s: &str) -> Result<MonitorKind, StorageError> {
    match s {
        "http" => Ok(MonitorKind::Http),
        "tcp" => Ok(MonitorKind::Tcp),
        "icmp" => Ok(MonitorKind::Icmp),
        "k8s_deployment" => Ok(MonitorKind::K8sDeployment),
        "k8s_stateful_set" => Ok(MonitorKind::K8sStatefulSet),
        "k8s_service" => Ok(MonitorKind::K8sService),
        "host_agent_check" => Ok(MonitorKind::HostAgentCheck),
        other => Err(StorageError::CorruptRow {
            detail: format!("monitors.kind holds unrecognized value {other:?}"),
        }),
    }
}

pub fn monitor_status_to_str(status: MonitorStatus) -> &'static str {
    match status {
        MonitorStatus::Pending => "pending",
        MonitorStatus::Up => "up",
        MonitorStatus::Down => "down",
        MonitorStatus::Paused => "paused",
        MonitorStatus::Stale => "stale",
    }
}

pub fn monitor_status_from_str(s: &str) -> Result<MonitorStatus, StorageError> {
    match s {
        "pending" => Ok(MonitorStatus::Pending),
        "up" => Ok(MonitorStatus::Up),
        "down" => Ok(MonitorStatus::Down),
        "paused" => Ok(MonitorStatus::Paused),
        "stale" => Ok(MonitorStatus::Stale),
        other => Err(StorageError::CorruptRow {
            detail: format!("status column holds unrecognized value {other:?}"),
        }),
    }
}

pub fn row_to_monitor(row: &Row) -> Result<Monitor, StorageError> {
    let id: i64 = row.get(0).map_err(query_failed)?;
    let name: String = row.get(1).map_err(query_failed)?;
    let target: String = row.get(2).map_err(query_failed)?;
    let kind: String = row.get(3).map_err(query_failed)?;
    let interval_secs: i64 = row.get(4).map_err(query_failed)?;
    let status: String = row.get(5).map_err(query_failed)?;
    let agent_id: Option<i64> = row.get(6).map_err(query_failed)?;
    Ok(Monitor {
        id: id as u64,
        name,
        target,
        kind: monitor_kind_from_str(&kind)?,
        interval_secs: interval_secs as u64,
        status: monitor_status_from_str(&status)?,
        agent_id: agent_id.map(|v| v as u64),
    })
}

pub fn row_to_check_result(row: &Row) -> Result<CheckResult, StorageError> {
    let monitor_id: i64 = row.get(0).map_err(query_failed)?;
    let checked_at: i64 = row.get(1).map_err(query_failed)?;
    let success: bool = row.get(2).map_err(query_failed)?;
    let latency_ms: i64 = row.get(3).map_err(query_failed)?;
    let message: Option<String> = row.get(4).map_err(query_failed)?;
    Ok(CheckResult {
        monitor_id: monitor_id as u64,
        checked_at: checked_at as u64,
        success,
        latency_ms: latency_ms as u64,
        message,
    })
}

pub fn row_to_agent(row: &Row) -> Result<Agent, StorageError> {
    let id: i64 = row.get(0).map_err(query_failed)?;
    let name: String = row.get(1).map_err(query_failed)?;
    let last_heartbeat_at: i64 = row.get(2).map_err(query_failed)?;
    let scope: String = row.get(3).map_err(query_failed)?;
    Ok(Agent {
        id: id as u64,
        name,
        last_heartbeat_at: last_heartbeat_at as u64,
        scope,
    })
}

pub fn row_to_alert_event(row: &Row) -> Result<AlertEvent, StorageError> {
    let id: i64 = row.get(0).map_err(query_failed)?;
    let monitor_id: i64 = row.get(1).map_err(query_failed)?;
    let transitioned_to: String = row.get(2).map_err(query_failed)?;
    let occurred_at: i64 = row.get(3).map_err(query_failed)?;
    let sinks_attempted: String = row.get(4).map_err(query_failed)?;
    let delivery_outcome: String = row.get(5).map_err(query_failed)?;
    Ok(AlertEvent {
        id: id as u64,
        monitor_id: monitor_id as u64,
        transitioned_to: monitor_status_from_str(&transitioned_to)?,
        occurred_at: occurred_at as u64,
        sinks_attempted,
        delivery_outcome,
    })
}
