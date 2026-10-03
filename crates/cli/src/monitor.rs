//! `monitra monitor …` — argument shape only (DESIGN.md §4 `cli`).

use clap::{Subcommand, ValueEnum};
use monitra_models::MonitorKind;

fn parse_interval(value: &str) -> Result<u64, String> {
    let seconds = value
        .parse::<u64>()
        .map_err(|_| "cli: monitor --interval must be an integer in 1..=86400".to_string())?;
    if monitra_models::valid_interval_secs(seconds) {
        Ok(seconds)
    } else {
        Err("cli: monitor --interval must be in 1..=86400".to_string())
    }
}

/// The clap-facing mirror of `monitra_models::MonitorKind`.
///
/// Kept separate from the domain type so `models` never depends on `clap` —
/// the same DTO/domain split DESIGN.md §4 uses between `backend`'s wire
/// types and `models`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MonitorKindArg {
    Http,
    Tcp,
    Icmp,
    K8sDeployment,
    K8sStatefulSet,
    K8sService,
    HostAgentCheck,
}

impl From<MonitorKindArg> for MonitorKind {
    fn from(arg: MonitorKindArg) -> Self {
        match arg {
            MonitorKindArg::Http => MonitorKind::Http,
            MonitorKindArg::Tcp => MonitorKind::Tcp,
            MonitorKindArg::Icmp => MonitorKind::Icmp,
            MonitorKindArg::K8sDeployment => MonitorKind::K8sDeployment,
            MonitorKindArg::K8sStatefulSet => MonitorKind::K8sStatefulSet,
            MonitorKindArg::K8sService => MonitorKind::K8sService,
            MonitorKindArg::HostAgentCheck => MonitorKind::HostAgentCheck,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum MonitorCommand {
    /// Register a new monitor.
    Add {
        #[arg(long)]
        name: String,
        /// URL, host:port, IP, or orchestrator-resource reference — meaning depends on `--kind`.
        #[arg(long)]
        target: String,
        #[arg(long, value_enum)]
        kind: MonitorKindArg,
        #[arg(long, value_parser = parse_interval)]
        interval: u64,
        /// The `Agent` this monitor depends on for its check data (Phase 6)
        /// — required for `host-agent-check` monitors, optional otherwise.
        #[arg(long)]
        agent_id: Option<u64>,
    },
    /// List all monitors.
    List,
    /// Show one monitor's detail.
    Show { id: u64 },
    /// Change a monitor's name, target, interval, or linked agent.
    Edit {
        id: u64,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        target: Option<String>,
        #[arg(long, value_parser = parse_interval)]
        interval: Option<u64>,
        #[arg(long)]
        agent_id: Option<u64>,
    },
    /// Delete a monitor.
    Remove { id: u64 },
    /// Pause checking a monitor.
    Pause { id: u64 },
    /// Resume a paused monitor (returns to `Pending`, not its pre-pause status — §5.3).
    Resume { id: u64 },
    /// Show a monitor's raw check-result history (Phase 9) — the same data
    /// the TUI/web Monitor detail screen reads.
    History {
        id: u64,
        /// Unix seconds — only results at or after this time.
        #[arg(long)]
        since: Option<u64>,
    },
    /// Per-region latency/failure-rate comparison across every target
    /// probed from more than one region-tagged agent (ADR-011, Phase 11).
    Regions {
        /// Unix seconds — only results at or after this time.
        #[arg(long)]
        since: Option<u64>,
    },
}

#[cfg(test)]
mod tests {
    use super::parse_interval;

    #[test]
    fn interval_rejects_out_of_range_values() {
        assert!(parse_interval("0").is_err());
        assert!(parse_interval("86401").is_err());
        assert!(parse_interval("18446744073709551615").is_err());
        assert_eq!(parse_interval("86400"), Ok(86400));
    }
}
