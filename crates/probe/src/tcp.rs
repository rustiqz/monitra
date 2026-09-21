//! TCP prober — `target` is `host:port`, success is a completed handshake
//! within `timeout` (DESIGN.md §5.1).

use std::time::{Duration, Instant};

use tokio::net::TcpStream;

use super::ProbeOutcome;

pub async fn probe(target: &str, timeout: Duration) -> ProbeOutcome {
    let started = Instant::now();
    match tokio::time::timeout(timeout, TcpStream::connect(target)).await {
        Ok(Ok(_stream)) => ProbeOutcome::Success {
            latency_ms: started.elapsed().as_millis() as u64,
        },
        Ok(Err(source)) => ProbeOutcome::Failure {
            message: source.to_string(),
        },
        Err(_elapsed) => ProbeOutcome::Failure {
            message: format!("timed out after {}ms", timeout.as_millis()),
        },
    }
}
