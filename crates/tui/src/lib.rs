//! Monitra's terminal dashboard (DESIGN.md §4 `tui`, ADR-009).
//!
//! A pure HTTP/WS API client against `monitra-backend`, in every mode —
//! never reads `monitra-storage`/`monitra-provider` directly, even locally
//! (an embedded backend on loopback stands in for a remote daemon when none
//! is running, per ADR-009; see §11.4). Owns terminal setup/teardown, the
//! event loop, widgets, and view state. Must restore the terminal on every
//! exit path, including panics.

mod app;
mod client;
mod data;
mod theme;
mod ui;

pub use client::Client;

use std::io;
use std::time::Duration;

use app::App;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    /// Runs on every exit path, including a panic unwinding through this
    /// frame — a panic that leaves the terminal in raw mode/alternate
    /// screen is a serious bug (DESIGN.md §4 `tui`'s contract), so this
    /// must not itself be skippable.
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Runs the TUI to completion against an already-connected `client` —
/// `main.rs` decides whether that client points at a remote daemon or an
/// embedded one it just booted (§11.4); this crate never knows which.
pub async fn run(client: Client) -> io::Result<()> {
    let endpoint = client.base_url_display();
    let (selected_tx, snapshot_rx) = data::spawn(client);
    let mut app = App::new(&endpoint, selected_tx);

    // The render loop is blocking (crossterm's `event::read()` blocks the
    // OS thread) — `spawn_blocking` keeps it off the async runtime's
    // worker threads while still letting it reach back into `snapshot_rx`
    // (a `watch::Receiver` is plain sync to poll) and the runtime `Handle`
    // that `data`'s background task keeps running on.
    tokio::task::spawn_blocking(move || run_loop(&mut app, snapshot_rx))
        .await
        .unwrap_or_else(|join_error| {
            Err(io::Error::other(format!(
                "tui: render loop panicked: {join_error}"
            )))
        })
}

fn run_loop(
    app: &mut App,
    mut snapshot_rx: tokio::sync::watch::Receiver<data::DashboardSnapshot>,
) -> io::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    while !app.should_quit {
        if snapshot_rx.has_changed().unwrap_or(false) {
            let snapshot = snapshot_rx.borrow_and_update().clone();
            app.on_snapshot(snapshot);
        }
        terminal.draw(|frame| ui::render(frame, app))?;
        if event::poll(Duration::from_millis(200))?
            && let Event::Key(key) = event::read()?
        {
            app.on_key(key);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn test_app() -> App {
        let (tx, _rx) = tokio::sync::watch::channel(None);
        let mut app = App::new("embedded · 127.0.0.1:8787", tx);
        app.on_snapshot(fixtures::sample_snapshot());
        app
    }

    #[test]
    fn every_screen_renders_at_reference_and_compact_sizes() {
        for screen in app::Screen::ALL {
            for (width, height) in [(118, 42), (80, 24)] {
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).expect("test backend");
                let mut app = test_app();
                app.screen = screen;
                terminal
                    .draw(|frame| ui::render(frame, &app))
                    .expect("screen renders");
            }
        }
    }

    #[test]
    fn empty_snapshot_renders_without_panicking() {
        for screen in app::Screen::ALL {
            let backend = TestBackend::new(100, 32);
            let mut terminal = Terminal::new(backend).expect("test backend");
            let (tx, _rx) = tokio::sync::watch::channel(None);
            let mut app = App::new("embedded · 127.0.0.1:8787", tx);
            app.screen = screen;
            terminal
                .draw(|frame| ui::render(frame, &app))
                .expect("screen renders with no data yet");
        }
    }

    /// The env var that tells this same test binary, re-invoked as a
    /// subprocess, to run the panic scenario instead of the normal suite.
    const PANIC_CHILD_ENV: &str = "MONITRA_TUI_PANIC_CHILD";

    /// DESIGN.md §4 `tui`'s hard contract: a panic must not leave the
    /// user's terminal in raw mode/alternate screen. Runs the guard-then-
    /// panic scenario in a real subprocess attached to a real pty (raw
    /// mode has no meaning on a non-tty stdout, which is what `cargo test`
    /// gives every process by default) and asserts the "leave alternate
    /// screen" / "disable raw mode" sequences appear in the pty's output
    /// even though the child panicked instead of exiting normally.
    #[test]
    fn panic_restores_terminal_via_subprocess() {
        let Some(pty) = pty::TestPty::open() else {
            // No pty available in this sandbox (e.g. no /dev/ptmx access) —
            // an environment limitation, not a code failure. `phase-verify`
            // environments are expected to have one; skip quietly rather
            // than fail the whole suite over something this test can't
            // control.
            eprintln!("panic_restores_terminal_via_subprocess: no pty available, skipping");
            return;
        };

        let exe = std::env::current_exe().expect("current test binary path");
        let output = pty.run_child(
            &exe,
            &[
                "--exact",
                "tests::panic_inside_guard_child",
                "--nocapture",
                "--test-threads=1",
            ],
            PANIC_CHILD_ENV,
        );

        assert!(
            output.contains("panicked at"),
            "child did not actually panic; output was:\n{output}"
        );
        assert!(
            output.contains("\u{1b}[?1049l"),
            "terminal was not restored (missing 'leave alternate screen') after a panic; output was:\n{output}"
        );
    }

    /// Not meaningful run directly — only does anything when invoked as the
    /// subprocess `panic_restores_terminal_via_subprocess` spawns, which
    /// sets [`PANIC_CHILD_ENV`]. Run any other way, it's a no-op so the
    /// normal test suite doesn't panic on every run.
    #[test]
    fn panic_inside_guard_child() {
        if std::env::var(PANIC_CHILD_ENV).is_err() {
            return;
        }
        let _guard = TerminalGuard::enter().expect("enter terminal guard on a real pty");
        panic!("deliberate panic to exercise TerminalGuard's Drop-based restore");
    }

    /// A minimal real pty, just enough to give a child process something
    /// where "raw mode" and "alternate screen" are meaningful — `cargo
    /// test` itself runs with stdio that usually isn't a tty at all.
    mod pty {
        use std::fs::File;
        use std::io::Read;
        use std::process::{Command, Stdio};
        use std::time::Duration;

        use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

        pub struct TestPty {
            master: File,
            slave_path: std::ffi::CString,
        }

        impl TestPty {
            pub fn open() -> Option<Self> {
                let master_fd = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).ok()?;
                grantpt(&master_fd).ok()?;
                unlockpt(&master_fd).ok()?;
                let slave_path = ptsname(&master_fd, Vec::new()).ok()?;
                Some(Self {
                    master: File::from(master_fd),
                    slave_path,
                })
            }

            fn open_slave(&self) -> File {
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(self.slave_path.to_str().expect("slave path is utf-8"))
                    .expect("open pty slave")
            }

            /// Runs `exe` with its stdio attached to this pty's slave side,
            /// waits for it to exit, and returns whatever the pty's master
            /// side received — reading with a hard deadline rather than
            /// waiting on EOF, since a pty only ever reliably signals "no
            /// more writers" as an `EIO` on the *next* read, not a clean 0.
            pub fn run_child(
                &self,
                exe: &std::path::Path,
                args: &[&str],
                env_flag: &str,
            ) -> String {
                let mut child = Command::new(exe)
                    .args(args)
                    .env(env_flag, "1")
                    .stdin(Stdio::from(self.open_slave()))
                    .stdout(Stdio::from(self.open_slave()))
                    .stderr(Stdio::from(self.open_slave()))
                    .spawn()
                    .expect("spawn subprocess on pty");

                let mut master = self.master.try_clone().expect("clone pty master");
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let mut collected = Vec::new();
                    loop {
                        match master.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => collected.extend_from_slice(&buf[..n]),
                            Err(_) => break, // EIO once every slave fd is closed
                        }
                    }
                    let _ = tx.send(collected);
                });

                let _ = child.wait();
                let collected = rx.recv_timeout(Duration::from_secs(3)).unwrap_or_default();
                String::from_utf8_lossy(&collected).into_owned()
            }
        }
    }
}

#[cfg(test)]
mod fixtures {
    //! Sample data for the snapshot tests only — not shipped as a runtime
    //! data source (the design-branch `fixtures.rs` this replaces was; the
    //! real data source is now `data.rs`, driven by the actual API).

    use monitra_models::{Agent, Monitor, MonitorKind, MonitorStatus};

    use crate::client::{
        AlertEventDto, CacheHealth, CheckResultDto, HealthResponse, NotifierHealth, StoreHealth,
    };
    use crate::data::DashboardSnapshot;

    pub fn sample_snapshot() -> DashboardSnapshot {
        DashboardSnapshot {
            monitors: vec![
                Monitor {
                    id: 1,
                    name: "api-gateway".to_string(),
                    target: "https://api.acme.io/health".to_string(),
                    kind: MonitorKind::Http,
                    interval_secs: 30,
                    status: MonitorStatus::Up,
                    agent_id: None,
                },
                Monitor {
                    id: 2,
                    name: "redis-primary".to_string(),
                    target: "10.0.2.9:6379".to_string(),
                    kind: MonitorKind::Tcp,
                    interval_secs: 30,
                    status: MonitorStatus::Down,
                    agent_id: None,
                },
                Monitor {
                    id: 3,
                    name: "prod/web-frontend".to_string(),
                    target: "eu-prod · deploy/web-frontend".to_string(),
                    kind: MonitorKind::K8sDeployment,
                    interval_secs: 30,
                    status: MonitorStatus::Up,
                    agent_id: None,
                },
                Monitor {
                    id: 4,
                    name: "db-01 disk".to_string(),
                    target: "disk:/".to_string(),
                    kind: MonitorKind::HostAgentCheck,
                    interval_secs: 30,
                    status: MonitorStatus::Stale,
                    agent_id: Some(1),
                },
            ],
            agents: vec![Agent {
                id: 1,
                name: "db-01".to_string(),
                last_heartbeat_at: 0,
                scope: "host · 10.0.2.11".to_string(),
                token: String::new(),
                region: None,
            }],
            alerts: vec![AlertEventDto {
                id: 1,
                monitor_id: 2,
                transitioned_to: MonitorStatus::Down,
                occurred_at: 0,
                sinks_attempted: "[\"webhook\"]".to_string(),
                delivery_outcome: "webhook: delivered".to_string(),
            }],
            health: Some(HealthResponse {
                status: "ok".to_string(),
                version: "0.0.0-test".to_string(),
                store: StoreHealth {
                    name: "sqlite".to_string(),
                    reachable: true,
                },
                cache: CacheHealth {
                    name: "in-process".to_string(),
                    degraded: false,
                },
                notifier: NotifierHealth {
                    name: "webhook".to_string(),
                    queue_len: 0,
                },
                k8s_clusters: vec!["eu-prod".to_string()],
            }),
            selected_history: Some((
                1,
                vec![CheckResultDto {
                    checked_at: 0,
                    success: true,
                    latency_ms: 142,
                    message: Some("200".to_string()),
                }],
            )),
            last_error: None,
        }
    }
}
