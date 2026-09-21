//! ICMP prober — `target` is a hostname or IP (DESIGN.md §5.1, §11.3).
//!
//! `surge-ping`'s default `Config` requests a `SOCK_DGRAM` ("unprivileged
//! ping") socket first and only falls back to `SOCK_RAW` (needs
//! `CAP_NET_RAW`) if that fails — exactly the §11.3 decision. If *both* fail
//! (locked-down environment, no ping-group-range grant, no capability), the
//! client never gets constructed and every ICMP probe on this host reports
//! `Unavailable`, never `Failure`/`Down` (P1) — the cause is on our side.

use std::net::IpAddr;
use std::time::Duration;

use surge_ping::{Client, Config, PingIdentifier, PingSequence, SurgeError};

use super::ProbeOutcome;

pub struct IcmpProber {
    client: Option<Client>,
}

impl Default for IcmpProber {
    fn default() -> Self {
        Self::new()
    }
}

impl IcmpProber {
    /// Never fails — a client that can't be constructed just means every
    /// future probe reports `Unavailable` instead of the engine refusing to
    /// start over one monitor kind (P1).
    pub fn new() -> Self {
        match Client::new(&Config::default()) {
            Ok(client) => Self {
                client: Some(client),
            },
            Err(source) => {
                tracing::warn!(
                    error = %source,
                    "probe: ICMP unavailable on this host (requires CAP_NET_RAW or a ping-group-range grant) — icmp monitors will report Unavailable"
                );
                Self { client: None }
            }
        }
    }

    pub async fn probe(&self, target: &str, timeout: Duration) -> ProbeOutcome {
        let Some(client) = &self.client else {
            return ProbeOutcome::Unavailable {
                message: "icmp unavailable on this host — requires CAP_NET_RAW".to_string(),
            };
        };

        let addr = match resolve(target).await {
            Ok(addr) => addr,
            Err(message) => return ProbeOutcome::Failure { message },
        };

        let mut pinger = client
            .pinger(addr, PingIdentifier(std::process::id() as u16))
            .await;
        pinger.timeout(timeout);

        match pinger.ping(PingSequence(0), &[]).await {
            Ok((_packet, duration)) => ProbeOutcome::Success {
                latency_ms: duration.as_millis() as u64,
            },
            Err(SurgeError::Timeout { .. }) => ProbeOutcome::Failure {
                message: format!("timed out after {}ms", timeout.as_millis()),
            },
            Err(SurgeError::IOError(source))
                if matches!(source.kind(), std::io::ErrorKind::PermissionDenied) =>
            {
                ProbeOutcome::Unavailable {
                    message: format!("icmp: permission denied: {source}"),
                }
            }
            Err(source) => ProbeOutcome::Failure {
                message: source.to_string(),
            },
        }
    }
}

async fn resolve(target: &str) -> Result<IpAddr, String> {
    if let Ok(addr) = target.parse::<IpAddr>() {
        return Ok(addr);
    }
    tokio::net::lookup_host((target, 0))
        .await
        .map_err(|source| format!("dns resolution failed: {source}"))?
        .next()
        .map(|sock_addr| sock_addr.ip())
        .ok_or_else(|| format!("no addresses found for {target}"))
}
