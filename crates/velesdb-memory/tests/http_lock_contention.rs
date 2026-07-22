//! A second `velesdb-memory --http` daemon on the same store (#1448-style
//! regression, HTTP variant).
//!
//! The store's single-writer `flock` (`velesdb-core`'s `Database::open_impl`)
//! guards cross-*process* access identically no matter which transport a
//! process serves over — HTTP is meant to let many CLIENTS share ONE daemon
//! *process*, not to let two daemon processes share one store. A second
//! `--http` process against a store another process already holds must fail
//! fast with the same actionable lock message the stdio path already has
//! (`mcp_lifecycle.rs`'s `database_locked_at_startup_prints_actionable_guidance_and_exits_nonzero`),
//! proving `open_store_with_actionable_lock_error` runs identically ahead of
//! either transport rather than being a stdio-only guard that regressed once
//! HTTP was added.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Upper bound on how long the first daemon is given to log that it's
/// listening (proof the store was opened and the HTTP listener bound) before
/// the test gives up and fails loudly instead of hanging.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bound on how long the contending second daemon is given to fail
/// and exit after hitting the locked store.
const CONTENDER_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

fn spawn_http_daemon(store_path: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
        .arg("--http")
        .arg("--http-port")
        .arg("0") // OS-assigned ephemeral port — this test only cares about
        // the STORE lock, never about a port collision between the two
        // daemons, so each gets its own free port.
        .env("VELESDB_MEMORY_PATH", store_path)
        .env("VELESDB_MEMORY_QUIET", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn velesdb-memory --http")
}

/// Read stderr lines on a helper thread until one contains `needle` (proof
/// the daemon reached that log line) or `timeout` elapses. Mirrors
/// `mcp_lifecycle.rs`'s `read_line_with_timeout` pattern: a direct blocking
/// read has no deadline, and if the process under test hangs, a bare
/// `read_to_string` would hang this test (and the suite) right along with
/// it.
fn wait_for_stderr_line(
    mut stderr: impl Read + Send + 'static,
    needle: &'static str,
    timeout: Duration,
) -> bool {
    use std::io::{BufRead, BufReader};

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(&mut stderr);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let found = line.contains(needle);
            let _ = tx.send(line);
            if found {
                return;
            }
        }
    });

    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match rx.recv_timeout(remaining) {
            Ok(line) if line.contains(needle) => return true,
            Ok(_) => {} // keep waiting for the needle line
            Err(_) => return false,
        }
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
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

fn drain_with_timeout<R: Read + Send + 'static>(mut reader: R, timeout: Duration) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = reader.read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    rx.recv_timeout(timeout).unwrap_or_default()
}

#[test]
fn second_http_daemon_on_same_store_fails_with_actionable_lock_message() {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");

    let mut holder = spawn_http_daemon(store_dir.path());
    let holder_stderr = holder.stderr.take().expect("holder stderr must be piped");
    let holder_up = wait_for_stderr_line(holder_stderr, "HTTP server listening", STARTUP_TIMEOUT);
    if !holder_up {
        let _ = holder.kill();
        let _ = holder.wait();
        panic!(
            "first HTTP daemon never logged its listening line within {STARTUP_TIMEOUT:?} — \
             it must be up (store opened, port bound) before this test can prove anything \
             about a second daemon contending on the same store"
        );
    }

    // Second daemon, same store, while the first is still up and holding the
    // flock — exactly the #1448 scenario, just over HTTP instead of stdio.
    let mut contender = spawn_http_daemon(store_dir.path());
    let contender_stderr = contender
        .stderr
        .take()
        .expect("contender stderr must be piped");
    drop(contender.stdout.take());

    let status = wait_for_exit(&mut contender, CONTENDER_EXIT_TIMEOUT).unwrap_or_else(|| {
        let _ = contender.kill();
        let _ = contender.wait();
        panic!(
            "a second --http daemon opening an already-locked store did not exit within \
             {CONTENDER_EXIT_TIMEOUT:?} — startup must fail fast (bounded retry), not hang, \
             exactly like the stdio path (#1448)"
        );
    });
    assert!(
        !status.success(),
        "a second --http daemon opening an already-locked store must exit non-zero, got: {status:?}"
    );

    let stderr_text = drain_with_timeout(contender_stderr, Duration::from_secs(2));
    let lower = stderr_text.to_lowercase();
    assert!(
        lower.contains("velesdb_memory_path") && lower.contains("pkill"),
        "expected the same actionable lock-contention message the stdio path prints \
         (naming VELESDB_MEMORY_PATH as the escape hatch and pkill as the fix), \
         got: {stderr_text:?}"
    );

    let _ = holder.kill();
    let _ = holder.wait();
}
