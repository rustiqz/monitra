//! Disk-space check (DESIGN.md §4 `agent`) — `statvfs` via `rustix`, no
//! subprocess and no polling library. `path` is any path on the filesystem
//! to check (typically a mount point); `min_free_pct` is the threshold
//! below which the check fails.

use std::time::Instant;

use super::CheckOutcome;

pub fn check_disk(path: &str, min_free_pct: f64) -> CheckOutcome {
    let started = Instant::now();
    match rustix::fs::statvfs(path) {
        Ok(stats) if stats.f_blocks == 0 => CheckOutcome::Unavailable {
            message: format!("agent: statvfs({path}) reported zero total blocks"),
        },
        Ok(stats) => {
            let latency_ms = started.elapsed().as_millis() as u64;
            let free_pct = (stats.f_bavail as f64 / stats.f_blocks as f64) * 100.0;
            if free_pct < min_free_pct {
                CheckOutcome::Failure {
                    message: format!(
                        "{path}: {free_pct:.1}% free, below the {min_free_pct:.1}% threshold"
                    ),
                }
            } else {
                CheckOutcome::Success { latency_ms }
            }
        }
        Err(err) => CheckOutcome::Unavailable {
            message: format!("agent: statvfs({path}) failed: {err}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_path_with_a_zero_threshold_succeeds() {
        let outcome = check_disk("/tmp", 0.0);
        assert!(matches!(outcome, CheckOutcome::Success { .. }));
    }

    #[test]
    fn an_unreachable_threshold_fails_without_claiming_unavailable() {
        let outcome = check_disk("/tmp", 100.1);
        assert!(matches!(outcome, CheckOutcome::Failure { .. }));
    }

    #[test]
    fn a_nonexistent_path_is_unavailable_not_a_failure() {
        let outcome = check_disk("/this/path/does/not/exist/on/any/host", 0.0);
        assert!(matches!(outcome, CheckOutcome::Unavailable { .. }));
    }
}
