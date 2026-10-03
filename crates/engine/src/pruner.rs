//! Low-priority, bounded raw-result retention (§5.4).

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use monitra_provider::Store;
use tokio::sync::watch;

const BATCH_SIZE: u64 = 10_000;
const MAX_BATCHES: u64 = 30;
const MAX_PASS_TIME: Duration = Duration::from_secs(2);
const YIELD_BETWEEN_BATCHES: Duration = Duration::from_millis(10);
const BACKLOG_ESTIMATE_CAP: u64 = 1_000_000;

#[derive(Debug, Clone, Copy)]
pub struct PruneConfig {
    pub retention_secs: u64,
    pub interval: Duration,
}

impl Default for PruneConfig {
    fn default() -> Self {
        Self {
            retention_secs: 7 * 86_400,
            interval: Duration::from_secs(600),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PassResult {
    rows: u64,
    backlog: u64,
    duration: Duration,
}

#[derive(Default)]
struct BehindTracker {
    last_pass_behind: bool,
}

impl BehindTracker {
    fn consecutive_backlog(&mut self, backlog: u64) -> bool {
        let warn = backlog > 0 && self.last_pass_behind;
        self.last_pass_behind = backlog > 0;
        warn
    }
}

async fn run_pass(
    store: &dyn Store,
    retention_secs: u64,
    max_batches: u64,
    max_time: Duration,
) -> Result<PassResult, monitra_provider::ProviderError> {
    let started = Instant::now();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let cutoff = now.saturating_sub(retention_secs);
    let mut rows = 0u64;
    let mut last_full = false;
    for _ in 0..max_batches {
        if started.elapsed() >= max_time {
            break;
        }
        let count = store
            .prune_check_results_older_than(cutoff, BATCH_SIZE)
            .await
            .map_err(|error| {
                tracing::warn!(rows_pruned = rows, duration_ms = started.elapsed().as_millis(), %error, "engine: result pruning pass failed during a batch");
                error
            })?;
        rows = rows.saturating_add(count);
        last_full = count == BATCH_SIZE;
        if !last_full {
            break;
        }
        tokio::time::sleep(YIELD_BETWEEN_BATCHES).await;
    }
    let backlog = if last_full {
        store
            .count_prunable_check_results(cutoff, BACKLOG_ESTIMATE_CAP)
            .await
            .map_err(|error| {
                tracing::warn!(rows_pruned = rows, duration_ms = started.elapsed().as_millis(), %error, "engine: result pruning pass failed counting backlog");
                error
            })?
    } else {
        0
    };
    Ok(PassResult {
        rows,
        backlog,
        duration: started.elapsed(),
    })
}

pub async fn run(
    store: Arc<dyn Store>,
    config: PruneConfig,
    mut ready: watch::Receiver<bool>,
    mut shutdown: watch::Receiver<bool>,
) {
    while !*ready.borrow() {
        tokio::select! {
            changed = ready.changed() => { if changed.is_err() { return; } }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return; }
            }
        }
    }
    let mut ticker = tokio::time::interval(config.interval.max(Duration::from_secs(1)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut behind = BehindTracker::default();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            _ = ticker.tick() => {
                match run_pass(store.as_ref(), config.retention_secs, MAX_BATCHES, MAX_PASS_TIME).await {
                    Ok(pass) => {
                        tracing::info!(rows_pruned = pass.rows, duration_ms = pass.duration.as_millis(), "engine: result pruning pass completed");
                        if behind.consecutive_backlog(pass.backlog) {
                            tracing::warn!(remaining_at_least = pass.backlog, estimate_capped_at = BACKLOG_ESTIMATE_CAP, "engine: result pruner is behind on consecutive passes");
                        }
                    }
                    Err(error) => {
                        behind = BehindTracker::default();
                        tracing::warn!(%error, "engine: result pruning pass failed");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_day_retention_matches_design() {
        assert_eq!(PruneConfig::default().retention_secs, 7 * 86_400);
    }

    #[test]
    fn consecutive_capped_passes_warn_and_recovery_resets_counter() {
        let mut tracker = BehindTracker::default();
        assert!(!tracker.consecutive_backlog(300_000));
        assert!(tracker.consecutive_backlog(200_000));
        assert!(!tracker.consecutive_backlog(0));
        assert!(!tracker.consecutive_backlog(10_000));
        assert!(tracker.consecutive_backlog(10_000));
    }
}
