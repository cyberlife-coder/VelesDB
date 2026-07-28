//! SIGTERM must drain the server, not kill it.
//!
//! `launchctl kickstart -k`, `systemctl restart` and `docker stop` all send
//! SIGTERM. Unhandled, it terminates the process outright and the
//! streamable-HTTP sessions clients hold are dropped mid-flight — the next
//! call on a live session hangs until the client's own timeout instead of
//! reconnecting. That is exactly what a daemon upgrade looked like from a
//! client's side before this was wired.
//!
//! The observable is the **exit code**, not the shutdown delay: an unhandled
//! signal also terminates instantly, so timing cannot tell the two apart. A
//! process killed by SIGTERM reports 143 (128 + 15); one that returned through
//! its own shutdown path reports 0.

#![cfg(unix)]

use std::net::TcpListener as StdTcpListener;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::time::Duration;

fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_velesdb-memory")
}

fn pick_free_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("read the bound address")
        .port()
}

#[test]
fn sigterm_shuts_the_http_server_down_through_its_own_path() {
    let store = tempfile::tempdir().expect("scratch store");
    // A scratch HOME too: the config file is looked up beside the store, and a
    // developer's own ~/.velesdb-memory must not decide what this test starts.
    let home = tempfile::tempdir().expect("scratch home");
    let port = pick_free_port();

    let mut child = Command::new(binary_path())
        .arg("--http")
        .arg("--http-insecure")
        .arg("--http-port")
        .arg(port.to_string())
        .env("HOME", home.path())
        .env("VELESDB_MEMORY_PATH", store.path())
        .env("VELESDB_MEMORY_QUIET", "1")
        .env_remove("VELESDB_MEMORY_CONFIG")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn velesdb-memory --http");

    // Wait for the server to actually be serving; sending the signal during
    // startup would prove nothing about the shutdown path.
    let mut serving = false;
    for _ in 0..40 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            serving = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(serving, "the daemon never accepted a connection on {port}");

    // SIGTERM, the signal every supervisor actually sends. Sent with `kill(1)`
    // rather than `libc::kill`: it needs neither an `unsafe` block nor a new
    // dev-dependency for a single call.
    let sent = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .expect("run kill(1)");
    assert!(sent.success(), "kill -TERM failed: {sent:?}");

    let status = child.wait().expect("wait for the daemon to exit");

    assert_eq!(
        status.signal(),
        None,
        "the daemon must not be terminated BY the signal — it must handle it \
         and return; being killed here is what drops live sessions"
    );
    assert_eq!(
        status.code(),
        Some(0),
        "a drained shutdown returns 0; 143 would mean SIGTERM killed the \
         process outright"
    );
}
