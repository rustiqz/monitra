//! Systemd unit-status check (DESIGN.md §4 `agent`) — shells out to
//! `systemctl is-active <unit>` rather than adding a dbus client
//! dependency; every systemd host already has `systemctl`, and every other
//! surface in this project already prefers "no new dependency" over one
//! more crate for a single call (CLAUDE.md).

use std::time::Instant;

use tokio::process::Command;

use monitra_probe::ProbeOutcome;

/// Recognized "the unit is not running" states. Anything else `systemctl`
/// prints (`"unknown"` for a unit that does not exist, or output this
/// version of `systemctl` was never checked against) is `Unavailable`, not
/// `Failure` — we cannot honestly claim the target is down if we do not
/// recognize what we were told (P1).
const DOWN_STATES: &[&str] = &[
    "inactive",
    "failed",
    "activating",
    "deactivating",
    "reloading",
];

pub async fn check_systemd(unit: &str) -> ProbeOutcome {
    check_systemd_with("systemctl", unit).await
}

async fn check_systemd_with(systemctl_bin: &str, unit: &str) -> ProbeOutcome {
    let started = Instant::now();
    let output = match Command::new(systemctl_bin)
        .arg("is-active")
        .arg(unit)
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) => {
            return ProbeOutcome::Unavailable {
                message: format!("agent: failed to run '{systemctl_bin} is-active {unit}': {err}"),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let latency_ms = started.elapsed().as_millis() as u64;

    if stdout == "active" {
        ProbeOutcome::Success { latency_ms }
    } else if DOWN_STATES.contains(&stdout.as_str()) {
        ProbeOutcome::Failure {
            message: format!("{unit}: systemctl reports '{stdout}'"),
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        ProbeOutcome::Unavailable {
            message: format!(
                "{unit}: systemctl returned an unrecognized status '{stdout}' (stderr: {stderr})"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// Serialises every test that writes a stub or spawns a process. A stub
    /// written by one test is still open for writing while another test's
    /// thread forks; the child inherits that handle until it execs, so
    /// executing the freshly written stub fails with ETXTBSY ("Text file
    /// busy") and the check reports `Unavailable`. Observed in CI and in
    /// about 3% of local runs of this module. The lock removes the overlap
    /// instead of retrying around it.
    static SPAWN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Writes a stub `systemctl` that ignores its arguments and just
    /// prints `stdout_line` — deterministic, no dependency on this test
    /// host actually running systemd.
    fn stub_systemctl(dir: &tempfile::TempDir, stdout_line: &str) -> std::path::PathBuf {
        let path = dir.path().join("systemctl");
        fs::write(&path, format!("#!/bin/sh\necho '{stdout_line}'\n")).expect("write stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    #[tokio::test]
    async fn active_is_success() {
        let _guard = SPAWN_LOCK.lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let stub = stub_systemctl(&dir, "active");
        let outcome = check_systemd_with(stub.to_str().unwrap(), "nginx.service").await;
        assert!(matches!(outcome, ProbeOutcome::Success { .. }));
    }

    #[tokio::test]
    async fn failed_is_a_failure_not_unavailable() {
        let _guard = SPAWN_LOCK.lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let stub = stub_systemctl(&dir, "failed");
        let outcome = check_systemd_with(stub.to_str().unwrap(), "nginx.service").await;
        assert!(matches!(outcome, ProbeOutcome::Failure { .. }));
    }

    #[tokio::test]
    async fn unrecognized_output_is_unavailable_not_a_failure() {
        let _guard = SPAWN_LOCK.lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let stub = stub_systemctl(&dir, "unknown");
        let outcome = check_systemd_with(stub.to_str().unwrap(), "nginx.service").await;
        assert!(matches!(outcome, ProbeOutcome::Unavailable { .. }));
    }

    #[tokio::test]
    async fn a_missing_systemctl_binary_is_unavailable() {
        // spawns too, so it must not fork while another test's stub is open
        let _guard = SPAWN_LOCK.lock().await;
        let outcome = check_systemd_with("/no/such/binary/systemctl", "nginx.service").await;
        assert!(matches!(outcome, ProbeOutcome::Unavailable { .. }));
    }
}
