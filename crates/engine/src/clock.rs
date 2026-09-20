//! §11.5: scheduling is anchored to `Instant` (monotonic — immune to clock
//! steps/DST), while `checked_at` timestamps use wall time. Mixing these up
//! produces either mass simultaneous checks or scrambled history, so every
//! wall-clock read in this crate goes through here.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `0` on a clock set before the Unix epoch — a misconfigured clock is a
/// distinct, surfaceable failure, not a reason to panic (P1: no
/// `unwrap()`/`expect()`).
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}
