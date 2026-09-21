//! The batched writer task (DESIGN.md §6.2, §11.1): the single most
//! important performance decision in the design. SQLite permits one writer;
//! funnelling every result through one task that batches inserts turns
//! thousands of individual transactions into a handful of batched ones.
//!
//! Bounded (§7.3): the channel has a fixed capacity. On overflow, `submit`
//! drops the newest result and logs loudly rather than blocking the caller
//! — a probe task must never stall waiting on the database.

use std::sync::Arc;
use std::time::Duration;

use monitra_models::CheckResult;
use monitra_provider::Store;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub struct Writer {
    tx: mpsc::Sender<CheckResult>,
}

impl Writer {
    /// Spawns the writer task and returns a handle to submit results plus
    /// the task's `JoinHandle` (awaited during graceful shutdown — §7.4 step
    /// 3, "flush the result buffer to disk").
    pub fn spawn(
        store: Arc<dyn Store>,
        capacity: usize,
        batch_size: usize,
        flush_interval: Duration,
    ) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(capacity);
        let handle = tokio::spawn(run(store, rx, batch_size, flush_interval));
        (Self { tx }, handle)
    }

    /// A handle with nothing consuming it — mirrors `Alerts::test_handle`,
    /// for other modules' tests (`scheduler.rs`) that need a `Writer` to
    /// construct their subject but don't care what happens to submitted
    /// results.
    #[cfg(test)]
    pub(crate) fn test_handle(capacity: usize) -> (Self, mpsc::Receiver<CheckResult>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (Self { tx }, rx)
    }

    /// Never blocks (§4.1 probe/write decoupling). Drops and logs loudly on
    /// a full queue rather than back-pressuring the caller.
    pub fn submit(&self, result: CheckResult) {
        let monitor_id = result.monitor_id;
        if let Err(source) = self.tx.try_send(result) {
            tracing::warn!(
                monitor_id,
                error = %source,
                "engine: writer queue full, dropping check result"
            );
        }
    }
}

async fn run(
    store: Arc<dyn Store>,
    mut rx: mpsc::Receiver<CheckResult>,
    batch_size: usize,
    flush_interval: Duration,
) {
    let mut batch = Vec::with_capacity(batch_size);
    let mut ticker = tokio::time::interval(flush_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            received = rx.recv() => {
                match received {
                    Some(result) => {
                        batch.push(result);
                        if batch.len() >= batch_size {
                            flush(&store, &mut batch).await;
                        }
                    }
                    // Every `Writer` (and thus every `Sender`) has been
                    // dropped — the engine is shutting down. Flush whatever
                    // is left and exit.
                    None => {
                        flush(&store, &mut batch).await;
                        return;
                    }
                }
            }
            _ = ticker.tick() => {
                flush(&store, &mut batch).await;
            }
        }
    }
}

async fn flush(store: &Arc<dyn Store>, batch: &mut Vec<CheckResult>) {
    if batch.is_empty() {
        return;
    }
    if let Err(source) = store.insert_check_results(batch).await {
        tracing::error!(
            count = batch.len(),
            error = %source,
            "engine: batched check-result write failed, results lost"
        );
    }
    batch.clear();
}
