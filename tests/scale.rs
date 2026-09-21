//! The §6.4 falsification harness — Phase 6's scaling hypothesis (§6.1) is
//! tested, not assumed. Lives at the workspace root, not in
//! `crates/engine/tests/`, because it needs a real `SqliteStore` wired to a
//! real `EngineHandle` end to end, and `monitra-engine` itself is
//! (correctly) forbidden by `scripts/dep-check.py` from depending on
//! `monitra-storage` — only the root binary is allowed to know about both
//! (CLAUDE.md's dependency DAG).
//!
//! `scale_smoke` runs in the routine `cargo test --workspace` gate (a few
//! seconds, N=10) and exists to prove the harness mechanism itself —
//! drift tracking, missed-check accounting, DB-write timing — is correct.
//! The full §6.4 protocol (N = 100/500/1000/2500/5000, 10 minutes each) is
//! `#[ignore]`d: run explicitly with
//! `cargo test --release --test scale -- --ignored --nocapture`, which
//! takes roughly 50 minutes end to end. Numbers from that run belong in the
//! README as the honest, measured scaling limit — never assumed from the
//! smoke test's numbers, which run too briefly and at too small an N to
//! mean anything at scale.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use monitra_models::{Agent, AlertEvent, CheckResult, Monitor, MonitorKind, MonitorStatus};
use monitra_provider::{ProviderError, Store};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Wraps a real `SqliteStore`, timing every batched `insert_check_results`
/// call — the §11.1 "highest risk" number this harness exists to produce.
/// Every other method is a plain delegate.
struct TimingStore {
    inner: monitra_storage::SqliteStore,
    write_latencies_ms: Mutex<Vec<f64>>,
}

#[async_trait]
impl Store for TimingStore {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        self.inner.health_check().await
    }

    async fn insert_monitor(&self, monitor: Monitor) -> Result<Monitor, ProviderError> {
        self.inner.insert_monitor(monitor).await
    }

    async fn get_monitor(&self, id: u64) -> Result<Option<Monitor>, ProviderError> {
        self.inner.get_monitor(id).await
    }

    async fn list_monitors(&self) -> Result<Vec<Monitor>, ProviderError> {
        self.inner.list_monitors().await
    }

    async fn update_monitor(
        &self,
        id: u64,
        name: Option<String>,
        target: Option<String>,
        interval_secs: Option<u64>,
        agent_id: Option<u64>,
    ) -> Result<(), ProviderError> {
        self.inner
            .update_monitor(id, name, target, interval_secs, agent_id)
            .await
    }

    async fn set_monitor_status(
        &self,
        id: u64,
        status: MonitorStatus,
    ) -> Result<(), ProviderError> {
        self.inner.set_monitor_status(id, status).await
    }

    async fn delete_monitor(&self, id: u64) -> Result<(), ProviderError> {
        self.inner.delete_monitor(id).await
    }

    async fn insert_check_results(&self, results: &[CheckResult]) -> Result<(), ProviderError> {
        let started = Instant::now();
        let outcome = self.inner.insert_check_results(results).await;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.write_latencies_ms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(elapsed_ms);
        outcome
    }

    async fn list_check_results(
        &self,
        monitor_id: u64,
        since: Option<u64>,
    ) -> Result<Vec<CheckResult>, ProviderError> {
        self.inner.list_check_results(monitor_id, since).await
    }

    async fn prune_check_results_older_than(
        &self,
        cutoff_unix_secs: u64,
    ) -> Result<u64, ProviderError> {
        self.inner
            .prune_check_results_older_than(cutoff_unix_secs)
            .await
    }

    async fn upsert_agent(&self, agent: Agent) -> Result<Agent, ProviderError> {
        self.inner.upsert_agent(agent).await
    }

    async fn heartbeat_agent(&self, id: u64, at_unix_secs: u64) -> Result<(), ProviderError> {
        self.inner.heartbeat_agent(id, at_unix_secs).await
    }

    async fn get_agent(&self, id: u64) -> Result<Option<Agent>, ProviderError> {
        self.inner.get_agent(id).await
    }

    async fn list_agents(&self) -> Result<Vec<Agent>, ProviderError> {
        self.inner.list_agents().await
    }

    async fn delete_agent(&self, id: u64) -> Result<(), ProviderError> {
        self.inner.delete_agent(id).await
    }

    async fn insert_alert_event(&self, event: AlertEvent) -> Result<AlertEvent, ProviderError> {
        self.inner.insert_alert_event(event).await
    }

    async fn list_alert_events(&self, monitor_id: u64) -> Result<Vec<AlertEvent>, ProviderError> {
        self.inner.list_alert_events(monitor_id).await
    }

    async fn list_all_alert_events(&self) -> Result<Vec<AlertEvent>, ProviderError> {
        self.inner.list_all_alert_events().await
    }
}

/// A bare-bones always-200 HTTP/1.1 responder — deliberately not a real
/// framework (axum etc.); at N=5000 that's 5000 app instances, real
/// overhead the harness itself shouldn't be adding.
async fn spawn_mock_target(latency: Duration) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock target");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(handle_mock_connection(socket, latency));
        }
    });
    addr
}

async fn handle_mock_connection(mut socket: TcpStream, latency: Duration) {
    let mut buf = [0u8; 512];
    let _ = socket.read(&mut buf).await;
    if latency > Duration::ZERO {
        tokio::time::sleep(latency).await;
    }
    let body = b"ok";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.write_all(body).await;
}

fn rss_mb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: f64 = rest
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse()
                .unwrap_or(0.0);
            return kb / 1024.0;
        }
    }
    0.0
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (((sorted.len() - 1) as f64) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[derive(Debug)]
struct ScaleReport {
    n: usize,
    actual_duration: Duration,
    p99_drift_ms: f64,
    max_drift_ms: f64,
    rss_mb_start: f64,
    rss_mb_end: f64,
    missed_checks: u64,
    expected_checks_per_monitor: u64,
    db_write_p50_ms: f64,
    db_write_p99_ms: f64,
}

fn print_report(label: &str, report: &ScaleReport) {
    println!(
        "[{label}] N={:<5} ran={:>6.1}s  expected/monitor={:<4} p99_drift={:>7.1}ms  \
         max_drift={:>7.1}ms  RSS {:.1}->{:.1}MB  missed={}  db_write p50/p99={:.2}/{:.2}ms",
        report.n,
        report.actual_duration.as_secs_f64(),
        report.expected_checks_per_monitor,
        report.p99_drift_ms,
        report.max_drift_ms,
        report.rss_mb_start,
        report.rss_mb_end,
        report.missed_checks,
        report.db_write_p50_ms,
        report.db_write_p99_ms,
    );
}

async fn run_scale(
    n: usize,
    interval: Duration,
    run_for: Duration,
    target_latency: Duration,
) -> ScaleReport {
    // `Monitor.interval_secs` is whole seconds (DESIGN.md §5.1) — rounding
    // here, once, keeps this function's own drift expectations consistent
    // with what the engine actually schedules, rather than silently
    // measuring against a sub-second interval nothing can honor.
    let interval = Duration::from_secs(interval.as_secs().max(1));

    let dir = tempfile::tempdir().expect("tempdir");
    let sqlite =
        monitra_storage::SqliteStore::open(dir.path().join("scale.db")).expect("open sqlite store");
    let timing_store = Arc::new(TimingStore {
        inner: sqlite,
        write_latencies_ms: Mutex::new(Vec::new()),
    });
    let store: Arc<dyn Store> = timing_store.clone();

    for i in 0..n {
        let addr = spawn_mock_target(target_latency).await;
        store
            .insert_monitor(Monitor {
                id: 0,
                name: format!("scale-{i}"),
                target: format!("http://{addr}/"),
                kind: MonitorKind::Http,
                interval_secs: interval.as_secs().max(1),
                status: MonitorStatus::Pending,
                agent_id: None,
            })
            .await
            .expect("insert monitor");
    }

    let config = monitra_engine::EngineConfig {
        scheduler: monitra_engine::SchedulerConfig {
            max_concurrent_probes: n.clamp(8, 512),
            probe_timeout: Duration::from_secs(5),
            // No live monitor mutation happens during a run — a long resync
            // interval keeps it from perturbing the drift measurement.
            resync_interval: Duration::from_secs(3600),
            shutdown_deadline: Duration::from_secs(10),
        },
        watchdog: monitra_engine::WatchdogConfig {
            check_interval: Duration::from_secs(3600),
            heartbeat_timeout: Duration::from_secs(3600),
        },
        alerts: monitra_engine::AlertConfig::default(),
        writer_capacity: (n * 4).max(4096),
        writer_batch_size: 200,
        writer_flush_interval: Duration::from_millis(200),
        results_channel_capacity: (n * 4).max(4096),
        ingest_capacity: (n * 4).max(4096),
        assignment_queue_capacity: 64,
    };

    // Log-only (no target attached) — this harness measures scheduling and
    // DB-write timing, not notifier delivery.
    let notifier = Arc::new(monitra_provider::RetryingNotifier::new(
        Arc::new(notify_webhook::WebhookNotifier::new(None)),
        64,
    ));

    let engine = monitra_engine::EngineHandle::start(
        monitra_engine::EngineDeps {
            store: Arc::clone(&store),
            k8s_factory: None,
            notifier,
        },
        config,
    );
    let mut results_rx = engine.subscribe();

    let rss_start = rss_mb();
    let start = Instant::now();
    let deadline = start + run_for;

    let mut first_seen: HashMap<u64, Instant> = HashMap::new();
    let mut seen_count: HashMap<u64, u64> = HashMap::new();
    let mut drift_samples_ms: Vec<f64> = Vec::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, results_rx.recv()).await {
            Ok(Ok(result)) => {
                let now = Instant::now();
                let first = *first_seen.entry(result.monitor_id).or_insert(now);
                let count = seen_count.entry(result.monitor_id).or_insert(0);
                if *count > 0 {
                    let expected_at = first + interval * (*count as u32);
                    let drift_ms =
                        now.saturating_duration_since(expected_at).as_secs_f64() * 1000.0;
                    drift_samples_ms.push(drift_ms);
                }
                *count += 1;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
            Err(_elapsed) => break,
        }
    }

    let rss_end = rss_mb();
    let actual_duration = start.elapsed();
    engine.shutdown().await;

    let expected_checks_per_monitor =
        (actual_duration.as_secs_f64() / interval.as_secs_f64()).floor() as u64;
    let missing_monitors = n.saturating_sub(seen_count.len()) as u64;
    let short_monitors = seen_count
        .values()
        // +1 slack: the boundary monitor that started right as the window
        // closed hasn't had time for its next check yet — not a miss.
        .filter(|&&count| count + 1 < expected_checks_per_monitor)
        .count() as u64;

    drift_samples_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut write_latencies_ms = timing_store
        .write_latencies_ms
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    write_latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

    ScaleReport {
        n,
        actual_duration,
        p99_drift_ms: percentile(&drift_samples_ms, 0.99),
        max_drift_ms: drift_samples_ms.last().copied().unwrap_or(0.0),
        rss_mb_start: rss_start,
        rss_mb_end: rss_end,
        missed_checks: missing_monitors + short_monitors,
        expected_checks_per_monitor,
        db_write_p50_ms: percentile(&write_latencies_ms, 0.50),
        db_write_p99_ms: percentile(&write_latencies_ms, 0.99),
    }
}

/// Proves the harness itself — drift tracking, missed-check accounting, and
/// DB-write timing — is correct, at a scale cheap enough for the routine
/// gate. Not evidence about the §6.1 hypothesis at any real N; see the
/// `#[ignore]`d tests below for that.
#[tokio::test]
async fn scale_smoke() {
    let report = run_scale(
        10,
        Duration::from_secs(1),
        Duration::from_secs(9),
        Duration::from_millis(5),
    )
    .await;
    print_report("smoke", &report);
    assert_eq!(
        report.missed_checks, 0,
        "smoke run should hit every scheduled check at N=10"
    );
    assert!(
        report.p99_drift_ms < 500.0,
        "p99 drift {}ms exceeds even this generous smoke-test bound",
        report.p99_drift_ms
    );
}

macro_rules! scale_test {
    ($name:ident, $n:expr) => {
        #[tokio::test]
        #[ignore = "§6.4 full protocol: 10 minutes per N — run explicitly with \
                    `cargo test --release --test scale -- --ignored --nocapture`"]
        async fn $name() {
            let report = run_scale(
                $n,
                Duration::from_secs(30),
                Duration::from_secs(600),
                Duration::from_millis(20),
            )
            .await;
            print_report(stringify!($name), &report);
            assert!(
                report.p99_drift_ms < 2000.0,
                "p99 drift {}ms exceeds the §6.4 2s bound at N={}",
                report.p99_drift_ms,
                $n
            );
            assert!(
                report.rss_mb_end < 512.0,
                "RSS {}MB exceeds the §6.4 512MB bound at N={}",
                report.rss_mb_end,
                $n
            );
            assert_eq!(
                report.missed_checks, 0,
                "N={} produced {} missed checks",
                $n, report.missed_checks
            );
        }
    };
}

scale_test!(scale_n100, 100);
scale_test!(scale_n500, 500);
scale_test!(scale_n1000, 1000);
scale_test!(scale_n2500, 2500);
scale_test!(scale_n5000, 5000);

/// A real (not simulated) but shortened run — actual scheduling, actual
/// SQLite writes, actual concurrent probes, just 60s instead of 10 minutes.
/// **Not** the §6.4 protocol and its numbers do not belong in the README as
/// the scaling limit — the full 10-minute tests above are the only
/// authoritative source for that. This exists as a fast pre-check anyone
/// can run (`cargo test --release --test scale -- --ignored preliminary`)
/// before committing to the full ~50-minute sweep.
#[tokio::test]
#[ignore = "real but shortened — see doc comment; not the §6.4 protocol"]
async fn preliminary_n500() {
    let report = run_scale(
        500,
        Duration::from_secs(30),
        Duration::from_secs(60),
        Duration::from_millis(20),
    )
    .await;
    print_report("preliminary_n500", &report);
}
