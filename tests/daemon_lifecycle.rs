use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "daemon-lifecycle-test-token";

fn spawn_daemon(home: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_monitra"))
        .args(["start", "--bind", "127.0.0.1:0"])
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("MONITRA_API_TOKEN", TOKEN)
        .env_remove("MONITRA_STORE")
        .env_remove("MONITRA_CACHE")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn monitra start")
}

fn run_service(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_monitra"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("MONITRA_STORE")
        .env_remove("MONITRA_CACHE")
        .output()
        .expect("run service command")
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn startup_line(child: &mut Child) -> String {
    let stderr = child.stderr.take().expect("piped stderr");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut seen = String::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = sender.send(seen);
                    break;
                }
                Ok(_) if line.contains("backend: listening") => {
                    let _ = sender.send(line);
                    break;
                }
                Ok(_) => seen.push_str(&line),
            }
        }
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("stderr produced no startup line within 5s")
}

#[test]
fn a_successful_start_logs_to_stderr() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut child = spawn_daemon(home.path());
    let line = startup_line(&mut child);
    let _ = child.kill();
    let _ = child.wait();
    assert!(line.contains("backend: listening"), "got: {line:?}");
}

#[test]
fn unsupported_attach_rejects_urls_without_leaking_credentials() {
    let home = tempfile::tempdir().expect("tempdir");
    for url in [
        "postgres://user:secret@host/db",
        "redis://user:secret@host:6379",
    ] {
        let output = run_service(home.path(), &["service", "attach", url]);
        assert!(!output.status.success());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(combined.contains("not supported yet"), "{combined}");
        assert!(!combined.contains("secret"));
        assert!(!combined.contains(url));
    }
}

#[test]
fn detach_legacy_provider_restores_startup_without_leaking_credentials() {
    for (category, url) in [
        ("store", "postgres://user:secret@host/db"),
        ("cache", "redis://user:secret@host:6379"),
    ] {
        let home = tempfile::tempdir().expect("tempdir");
        let config_dir = home.path().join(".config/monitra");
        std::fs::create_dir_all(&config_dir).expect("config directory");
        std::fs::write(
            config_dir.join("config.toml"),
            format!("{category} = \"{url}\"\n"),
        )
        .expect("legacy config");

        let mut child = spawn_daemon(home.path());
        let status = wait_with_timeout(&mut child, Duration::from_secs(5));
        if status.is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        assert!(
            status.is_some_and(|status| !status.success()),
            "startup should reject {category}"
        );
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .expect("piped stderr")
            .read_to_string(&mut stderr)
            .expect("read startup error");
        assert!(stderr.contains("unsupported"), "{stderr}");
        assert!(!stderr.contains("secret"));
        assert!(!stderr.contains(url));
        assert!(!stderr.contains("degrading"));

        let output = run_service(home.path(), &["service", "detach", category]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut child = spawn_daemon(home.path());
        let line = startup_line(&mut child);
        let _ = child.kill();
        let _ = child.wait();
        assert!(line.contains("backend: listening"), "{line}");
    }
}

#[test]
#[cfg(unix)]
fn sigterm_exits_promptly_and_closes_ws_cleanly() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut child = spawn_daemon(home.path());
    let line = startup_line(&mut child);
    let addr = line
        .split("addr=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("bound address in startup log")
        .trim_matches('"');

    let mut socket = TcpStream::connect(addr).expect("connect to /ws");
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {TOKEN}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    socket
        .write_all(request.as_bytes())
        .expect("send upgrade request");
    let mut response = [0u8; 1024];
    let length = socket.read(&mut response).expect("read handshake");
    assert!(String::from_utf8_lossy(&response[..length]).starts_with("HTTP/1.1 101"));

    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let exited = wait_with_timeout(&mut child, Duration::from_secs(5));
    assert!(exited.is_some(), "daemon did not exit within 5s of SIGTERM");
    let mut close_frame = [0u8; 64];
    match socket.read(&mut close_frame) {
        Ok(0) => {}
        Ok(length) => assert_eq!(
            close_frame[0] & 0x0f,
            0x8,
            "unexpected frame: {:?}",
            &close_frame[..length]
        ),
        Err(error) => panic!("socket did not close cleanly: {error}"),
    }
}
