//! Process-liveness check (DESIGN.md §4 `agent`) — scans `/proc` directly,
//! no dependency on a system-inventory crate. `name` is matched against
//! each process's `/proc/<pid>/comm`, trimmed — note `comm` truncates at 15
//! bytes (a kernel limit), so a longer executable name only needs to match
//! on its first 15 bytes.

use std::fs;
use std::path::Path;
use std::time::Instant;

use monitra_probe::ProbeOutcome;

pub fn check_process(name: &str) -> ProbeOutcome {
    check_process_in(Path::new("/proc"), name)
}

/// `proc_dir` is injectable so tests can point this at a fixture directory
/// instead of the real `/proc` — deterministic, no dependency on which
/// processes happen to be running wherever this test executes.
fn check_process_in(proc_dir: &Path, name: &str) -> ProbeOutcome {
    let started = Instant::now();
    let entries = match fs::read_dir(proc_dir) {
        Ok(entries) => entries,
        Err(err) => {
            return ProbeOutcome::Unavailable {
                message: format!("agent: failed to read {}: {err}", proc_dir.display()),
            };
        }
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(pid_str) = file_name.to_str() else {
            continue;
        };
        if pid_str.parse::<u32>().is_err() {
            continue;
        }
        let comm_path = entry.path().join("comm");
        // The process may have exited between listing and reading — not
        // our problem to report, just skip it and keep scanning.
        let Ok(comm) = fs::read_to_string(&comm_path) else {
            continue;
        };
        if comm.trim() == name {
            let latency_ms = started.elapsed().as_millis() as u64;
            return ProbeOutcome::Success { latency_ms };
        }
    }

    ProbeOutcome::Failure {
        message: format!("no running process named '{name}'"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn fixture_proc(entries: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for (pid, comm) in entries {
            let pid_dir = dir.path().join(pid);
            fs::create_dir(&pid_dir).expect("create pid dir");
            fs::write(pid_dir.join("comm"), format!("{comm}\n")).expect("write comm");
        }
        // A non-numeric entry (e.g. "self", "net") must be skipped, not
        // mistaken for a pid.
        fs::create_dir(dir.path().join("self")).expect("create self dir");
        dir
    }

    #[test]
    fn matching_comm_succeeds() {
        let dir = fixture_proc(&[("1", "systemd"), ("42", "postgres")]);
        let outcome = check_process_in(dir.path(), "postgres");
        assert!(matches!(outcome, ProbeOutcome::Success { .. }));
    }

    #[test]
    fn no_matching_comm_fails_not_unavailable() {
        let dir = fixture_proc(&[("1", "systemd")]);
        let outcome = check_process_in(dir.path(), "nonexistent-daemon");
        assert!(matches!(outcome, ProbeOutcome::Failure { .. }));
    }

    #[test]
    fn unreadable_proc_dir_is_unavailable() {
        let outcome = check_process_in(Path::new("/this/does/not/exist/anywhere"), "anything");
        assert!(matches!(outcome, ProbeOutcome::Unavailable { .. }));
    }
}
