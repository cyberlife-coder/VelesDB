//! Process-level proof of the `--http-insecure` /
//! `VELESDB_MEMORY_HTTP_INSECURE=1` escape hatch: HTTPS-by-default is
//! decided in `src/main.rs`, BEFORE `velesdb_memory::http::router` is ever
//! built, so an in-process test against the router (as `tests/http_tls.rs`
//! and `tests/http_transport.rs` use) can't exercise that decision — only
//! spawning the actual compiled binary can.
//!
//! Anti-hang / anti-orphan discipline: every subprocess spawned here is
//! wrapped in a guard that SIGKILLs and reaps it on drop (including on test
//! panic, via the guard's `Drop` impl), and every wait on the daemon
//! becoming ready is bounded by an explicit timeout — never an unbounded
//! poll loop.

use std::io::{BufRead, BufReader};
use std::net::TcpListener as StdTcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Owns a spawned `velesdb-memory` child process and guarantees it is
/// killed and reaped when dropped — including when a test panics before
/// reaching its own explicit cleanup, so a failing assertion here can never
/// leak a daemon that keeps listening on the test's port.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn a thread that continuously drains `child`'s stderr into a shared
/// buffer and return a handle to read it back at any point.
///
/// This is NOT optional plumbing: `child` is a long-running daemon that
/// never closes its stderr on its own, so a synchronous
/// `stderr.read_to_string(..)` call would block until EOF — i.e. forever,
/// for as long as the daemon stays up. Draining continuously on a
/// background thread instead means (a) the child's stderr pipe never fills
/// up and makes IT block on a write(), and (b) this test can inspect
/// whatever has been captured so far without ever blocking on the child's
/// lifetime.
fn drain_stderr(child: &mut Child) -> Arc<Mutex<String>> {
    let buffer = Arc::new(Mutex::new(String::new()));
    let stderr = child
        .stderr
        .take()
        .expect("child spawned with Stdio::piped() stderr");
    let buffer_writer = Arc::clone(&buffer);
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Ok(mut buffer) = buffer_writer.lock() {
                buffer.push_str(&line);
                buffer.push('\n');
            }
        }
    });
    buffer
}

/// Bind an OS-assigned loopback port, read it back, then release it
/// immediately so the subprocess can bind the same port — a brief race in
/// theory, but the same "ask the OS for a free port, then reuse the number"
/// pattern process-level tests use throughout this codebase (there is no
/// portable way to hand a `--http-port 0`-launched *subprocess*'s
/// OS-assigned port back to the parent other than parsing its stderr, which
/// `serve_http`'s current banner does not support for `bind_addr:0`).
fn pick_free_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("bind ephemeral port to pick one");
    listener.local_addr().expect("read bound local addr").port()
}

/// Locate the `velesdb-memory` binary built by `cargo test` for this crate
/// (same directory Cargo places integration test binaries' sibling `deps/`
/// output next to). `env!("CARGO_BIN_EXE_velesdb-memory")` is Cargo's own
/// supported mechanism for exactly this — it also guarantees the binary is
/// built with the SAME feature set this test binary was compiled with
/// (`--features http`), since `required-features = ["http"]` on this test
/// target means it never even builds without it.
fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_velesdb-memory")
}

/// Poll `http://127.0.0.1:{port}/health` until it answers or `timeout`
/// elapses. Bounded — never an unbounded loop — so a daemon that fails to
/// start turns into a clear test failure instead of a hang.
fn wait_for_plain_http_health(port: u16, timeout: Duration) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    let mut last_err = String::from("never attempted");
    while std::time::Instant::now() < deadline {
        match reqwest::blocking::get(format!("http://127.0.0.1:{port}/health")) {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => last_err = format!("non-success status: {}", response.status()),
            Err(err) => last_err = err.to_string(),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("daemon never answered /health in time: {last_err}"))
}

/// `--http-insecure` must serve plain HTTP (no TLS) despite HTTPS now being
/// the default — the documented fallback for local debugging or when a
/// trusted TLS-terminating proxy already sits in front (see the
/// `--http-insecure` doc comment on `requested_http_bind` in `src/main.rs`).
#[test]
fn http_insecure_flag_serves_plain_http() {
    let port = pick_free_port();
    let store_dir = tempfile::tempdir().expect("create scratch store dir");

    let mut child = Command::new(binary_path())
        .arg("--http")
        .arg("--http-insecure")
        .arg("--http-port")
        .arg(port.to_string())
        .env("VELESDB_MEMORY_PATH", store_dir.path())
        .env("VELESDB_MEMORY_QUIET", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn velesdb-memory --http --http-insecure");
    let stderr_output = drain_stderr(&mut child);
    let mut guard = ChildGuard(child);

    let ready = wait_for_plain_http_health(port, Duration::from_secs(10));

    // A plain (non-TLS) GET succeeding is the actual proof; the drained
    // stderr (daemon startup banner, any bind error) is only for a useful
    // failure message.
    let stderr_output = stderr_output.lock().expect("stderr buffer lock").clone();

    assert!(
        ready.is_ok(),
        "expected --http-insecure to serve plain HTTP reachable at /health: {ready:?}\nstderr:\n{stderr_output}"
    );

    // Explicit shutdown before the guard's Drop runs too, so a failure to
    // terminate surfaces as part of THIS test rather than silently in a
    // background drop.
    guard.0.kill().expect("kill the insecure daemon");
    guard
        .0
        .wait()
        .expect("reap the insecure daemon after kill — no orphan process");
}
