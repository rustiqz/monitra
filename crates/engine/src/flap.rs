//! Flap damping (DESIGN.md §5.3): transitions between `Up` and `Down`
//! require *N* consecutive results in the new state, not a single one.
//! Default N=2 for down-transitions, N=1 for recovery — asymmetric because
//! we want to be slow to alarm and fast to reassure.

use monitra_models::MonitorStatus;

pub const DOWN_THRESHOLD: u32 = 2;
pub const UP_THRESHOLD: u32 = 1;

/// Per-monitor flap-damping counters. Reset whenever the observed outcome
/// flips, so a single blip doesn't slowly accumulate toward a transition it
/// shouldn't cause.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlapState {
    consecutive_success: u32,
    consecutive_failure: u32,
}

impl FlapState {
    /// Records one probe outcome against `current` status and returns the
    /// new status if this observation should *cause* a transition, or
    /// `None` if `current` should hold. Never called for `Unavailable`
    /// outcomes — those go straight to `Stale` without touching the
    /// counters (§5.1: staleness isn't a flap-damped up/down signal).
    pub fn observe(&mut self, success: bool, current: MonitorStatus) -> Option<MonitorStatus> {
        if success {
            self.consecutive_failure = 0;
            self.consecutive_success += 1;
        } else {
            self.consecutive_success = 0;
            self.consecutive_failure += 1;
        }

        match (success, current) {
            // Pending/Stale have never been confirmed either way — the
            // first result decides, no damping needed.
            (true, MonitorStatus::Up) => None,
            (true, _) if self.consecutive_success >= UP_THRESHOLD => Some(MonitorStatus::Up),
            (false, MonitorStatus::Down) => None,
            (false, _) if self.consecutive_failure >= DOWN_THRESHOLD => Some(MonitorStatus::Down),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn down_transition_needs_two_consecutive_failures() {
        let mut flap = FlapState::default();
        assert_eq!(flap.observe(false, MonitorStatus::Up), None);
        assert_eq!(
            flap.observe(false, MonitorStatus::Up),
            Some(MonitorStatus::Down)
        );
    }

    #[test]
    fn single_dropped_packet_does_not_page_anyone() {
        let mut flap = FlapState::default();
        assert_eq!(flap.observe(false, MonitorStatus::Up), None);
        // Recovers before the second consecutive failure — no transition.
        assert_eq!(flap.observe(true, MonitorStatus::Up), None);
    }

    #[test]
    fn up_transition_is_immediate() {
        let mut flap = FlapState::default();
        assert_eq!(
            flap.observe(true, MonitorStatus::Down),
            Some(MonitorStatus::Up)
        );
    }

    #[test]
    fn pending_resolves_on_first_result() {
        let mut flap = FlapState::default();
        assert_eq!(
            flap.observe(true, MonitorStatus::Pending),
            Some(MonitorStatus::Up)
        );

        let mut flap = FlapState::default();
        assert_eq!(flap.observe(false, MonitorStatus::Pending), None);
        assert_eq!(
            flap.observe(false, MonitorStatus::Pending),
            Some(MonitorStatus::Down)
        );
    }

    #[test]
    fn already_down_does_not_re_signal_down() {
        let mut flap = FlapState::default();
        assert_eq!(flap.observe(false, MonitorStatus::Down), None);
        assert_eq!(flap.observe(false, MonitorStatus::Down), None);
    }

    #[test]
    fn already_up_does_not_re_signal_up() {
        let mut flap = FlapState::default();
        assert_eq!(flap.observe(true, MonitorStatus::Up), None);
    }
}
