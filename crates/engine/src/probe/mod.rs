//! Per-`MonitorKind` probe execution (DESIGN.md §4 `engine`, §6.3).
//!
//! Every prober returns a [`ProbeOutcome`] rather than a `Result` — a probe
//! failing to reach its target is the expected, common case, not an error
//! (P1: never let a single monitor's trouble propagate as an engine error).

mod http;
mod icmp;
mod tcp;

use std::time::Duration;

use monitra_models::MonitorKind;

pub use icmp::IcmpProber;

/// The result of one probe attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    Success {
        latency_ms: u64,
    },
    Failure {
        message: String,
    },
    /// The probe could not determine target health because of a problem on
    /// *our* side (e.g. missing `CAP_NET_RAW` for ICMP) — never collapsed
    /// into `Failure`/`Down` (§11.3, P1).
    Unavailable {
        message: String,
    },
}

/// The subset of `MonitorKind` the scheduler dispatches to a network probe
/// directly. `K8s*` kinds go through a `Collector` instead (§3.3);
/// `HostAgentCheck` is fed by pushed agent results (Phase 8) and is never
/// scheduler-dispatched at all. `TryFrom` makes that split a compile-time
/// exhaustive match rather than a runtime `unreachable!()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkProbeKind {
    Http,
    Tcp,
    Icmp,
}

impl TryFrom<MonitorKind> for NetworkProbeKind {
    type Error = ();

    fn try_from(kind: MonitorKind) -> Result<Self, Self::Error> {
        match kind {
            MonitorKind::Http => Ok(NetworkProbeKind::Http),
            MonitorKind::Tcp => Ok(NetworkProbeKind::Tcp),
            MonitorKind::Icmp => Ok(NetworkProbeKind::Icmp),
            MonitorKind::K8sDeployment
            | MonitorKind::K8sStatefulSet
            | MonitorKind::K8sService
            | MonitorKind::HostAgentCheck => Err(()),
        }
    }
}

/// Shared, reusable clients for the network probers — created once at
/// engine startup, not per probe (a fresh `reqwest::Client`/ICMP socket per
/// check would defeat connection reuse and exhaust file descriptors at
/// scale, §6.1).
pub struct Probers {
    http_client: reqwest::Client,
    icmp: IcmpProber,
}

impl Probers {
    pub fn new(icmp: IcmpProber) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            icmp,
        }
    }

    /// Runs one probe with a hard timeout (§6.3 point 3) — a hung connection
    /// to one target can never stall the caller past `timeout`.
    pub async fn probe(
        &self,
        kind: NetworkProbeKind,
        target: &str,
        timeout: Duration,
    ) -> ProbeOutcome {
        match kind {
            NetworkProbeKind::Http => http::probe(&self.http_client, target, timeout).await,
            NetworkProbeKind::Tcp => tcp::probe(target, timeout).await,
            NetworkProbeKind::Icmp => self.icmp.probe(target, timeout).await,
        }
    }
}
