//! Bounded local buffer for results awaiting a successful push (DESIGN.md
//! §7.3, §4 `agent`) — mirrors `RetryingNotifier`
//! (`crates/provider/src/policy.rs`) in shape (drop-oldest-and-log on
//! overflow) but reimplemented locally: `monitra-agent` may never depend
//! on `monitra-provider` (DAG, ADR-008).

use std::collections::VecDeque;

use crate::client::PendingResult;

pub struct PushBuffer {
    queue: VecDeque<PendingResult>,
    capacity: usize,
}

impl PushBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Buffers `results` (typically everything from a failed push),
    /// dropping the oldest buffered entry first whenever the queue is
    /// already at capacity — never blocks, never grows unbounded (§7.3).
    pub fn push_back_all(&mut self, results: Vec<PendingResult>) {
        for result in results {
            if self.queue.len() >= self.capacity {
                let dropped = self.queue.pop_front();
                tracing::warn!(
                    dropped_monitor_id = dropped.as_ref().map(|d| d.monitor_id),
                    capacity = self.capacity,
                    "agent: push buffer full, dropping oldest buffered result"
                );
            }
            self.queue.push_back(result);
        }
    }

    /// Drains everything currently buffered, oldest first — the caller
    /// prepends this to the next cycle's fresh results before attempting
    /// another push.
    pub fn drain(&mut self) -> Vec<PendingResult> {
        self.queue.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckOutcome;

    fn result(monitor_id: u64) -> PendingResult {
        PendingResult {
            monitor_id,
            outcome: CheckOutcome::Success { latency_ms: 1 },
        }
    }

    #[test]
    fn buffers_up_to_capacity_without_dropping() {
        let mut buffer = PushBuffer::new(3);
        buffer.push_back_all(vec![result(1), result(2), result(3)]);
        assert_eq!(buffer.len(), 3);
        let drained: Vec<u64> = buffer.drain().iter().map(|r| r.monitor_id).collect();
        assert_eq!(drained, vec![1, 2, 3]);
    }

    #[test]
    fn overflow_drops_oldest_first_not_newest() {
        let mut buffer = PushBuffer::new(2);
        buffer.push_back_all(vec![result(1), result(2), result(3)]);
        assert_eq!(buffer.len(), 2);
        let drained: Vec<u64> = buffer.drain().iter().map(|r| r.monitor_id).collect();
        assert_eq!(
            drained,
            vec![2, 3],
            "oldest (1) must be dropped, not newest"
        );
    }

    #[test]
    fn drain_leaves_the_buffer_empty() {
        let mut buffer = PushBuffer::new(2);
        buffer.push_back_all(vec![result(1)]);
        buffer.drain();
        assert!(buffer.is_empty());
    }
}
