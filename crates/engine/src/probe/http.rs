//! HTTP prober — `target` is a URL, success is a 2xx/3xx response within
//! `timeout` (DESIGN.md §5.1).

use std::time::{Duration, Instant};

use super::ProbeOutcome;

pub async fn probe(client: &reqwest::Client, target: &str, timeout: Duration) -> ProbeOutcome {
    let started = Instant::now();
    let result = client.get(target).timeout(timeout).send().await;
    let latency_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(response) if response.status().is_success() || response.status().is_redirection() => {
            ProbeOutcome::Success { latency_ms }
        }
        Ok(response) => ProbeOutcome::Failure {
            message: format!("unexpected status {}", response.status()),
        },
        Err(source) if source.is_timeout() => ProbeOutcome::Failure {
            message: format!("timed out after {}ms", timeout.as_millis()),
        },
        Err(source) => ProbeOutcome::Failure {
            message: source.to_string(),
        },
    }
}
