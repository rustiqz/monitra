//! Hard-timeout gate (DESIGN.md §6.3 point 3, Phase 6 acceptance): a hung
//! connection to one target must never be able to stall the caller past its
//! configured timeout. `10.255.255.1` is a non-routable test address —
//! connection attempts to it neither succeed nor get actively refused, they
//! just hang, which is exactly the failure mode a probe's own timeout (not
//! the OS's much longer TCP retry timeout) has to catch.

use std::time::{Duration, Instant};

use monitra_engine::probe::{IcmpProber, NetworkProbeKind, ProbeOutcome, Probers};

const UNROUTABLE: &str = "10.255.255.1";

#[tokio::test]
async fn tcp_probe_returns_within_its_own_timeout_against_a_hanging_target() {
    let timeout = Duration::from_millis(300);
    let started = Instant::now();

    let probers = Probers::new(IcmpProber::new());
    let outcome = probers
        .probe(NetworkProbeKind::Tcp, &format!("{UNROUTABLE}:81"), timeout)
        .await;

    let elapsed = started.elapsed();
    assert!(
        elapsed < timeout + Duration::from_secs(2),
        "probe took {elapsed:?}, far longer than its {timeout:?} timeout — the hard timeout did not bound it"
    );
    assert!(
        matches!(outcome, ProbeOutcome::Failure { .. }),
        "expected a bounded Failure outcome, got {outcome:?}"
    );
}

#[tokio::test]
async fn http_probe_returns_within_its_own_timeout_against_a_hanging_target() {
    let timeout = Duration::from_millis(300);
    let started = Instant::now();

    let probers = Probers::new(IcmpProber::new());
    let outcome = probers
        .probe(
            NetworkProbeKind::Http,
            &format!("http://{UNROUTABLE}/"),
            timeout,
        )
        .await;

    let elapsed = started.elapsed();
    assert!(
        elapsed < timeout + Duration::from_secs(2),
        "probe took {elapsed:?}, far longer than its {timeout:?} timeout — the hard timeout did not bound it"
    );
    assert!(
        matches!(outcome, ProbeOutcome::Failure { .. }),
        "expected a bounded Failure outcome, got {outcome:?}"
    );
}
