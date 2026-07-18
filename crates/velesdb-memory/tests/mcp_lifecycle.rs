//! MCP server process lifecycle (#1448).
//!
//! Regression this catches: the `velesdb-memory` binary must exit — and
//! release its single-writer store lock — when its stdio transport reaches
//! EOF, exactly as any well-behaved MCP stdio server does when its client
//! disconnects. Before the fix, closing stdin left the process running
//! forever (`running.waiting()` never resolved because nothing observed
//! the transport's EOF), so every disconnect (a `claude mcp list`
//! health-check, a closed Claude Code session) leaked a zombie that kept
//! holding the store lock — and the *next* session's `initialize` failed
//! with `Storage(DatabaseLocked)`, surfaced to the user as an opaque
//! "Failed to connect".
//!
//! Spawns the real binary (`env!("CARGO_BIN_EXE_velesdb-memory")`), talks
//! one `initialize` over stdin/stdout, closes stdin, and asserts the
//! process exits within a bounded timeout — then proves the lock was
//! actually released by successfully `initialize`-ing a second process on
//! the same store path.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

/// Upper bound the test is willing to wait for a clean shutdown. The MCP
/// stdio lifecycle is local-process and near-instantaneous, so 5s is already
/// generous headroom over any plausible tokio/runtime teardown cost.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Read one line from `reader` on a helper thread and wait for it at most
/// `timeout`. `BufRead::read_line` has no built-in deadline, and if the
/// process under test is the one hanging (exactly what #1448 is about), a
/// direct blocking read would hang this test — and the whole suite — right
/// along with it. Returns `None` on timeout, leaving the helper thread
/// (and its now-orphaned read) to be cleaned up when the child is killed
/// and its pipe closed.
fn read_line_with_timeout(mut reader: BufReader<ChildStdout>, timeout: Duration) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = reader.read_line(&mut line).map(|n| (n, line));
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        // 0 bytes = EOF before any line arrived; read/recv errors read the same.
        Ok(Ok((0, _)) | Err(_)) | Err(_) => None,
        Ok(Ok((_, line))) => Some(line),
    }
}

fn initialize_request(id: u64) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"mcp_lifecycle_test","version":"0"}}}}}}"#
    )
}

/// Spawn the server binary against `store_path`, pointed at a scratch store
/// so this test never touches the user's real `~/.velesdb-memory`.
fn spawn_server(store_path: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
        .env("VELESDB_MEMORY_PATH", store_path)
        .env("VELESDB_MEMORY_QUIET", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn velesdb-memory binary")
}

/// Send one `initialize` request and block until the matching JSON-RPC
/// response line comes back on stdout — the request/response round trip
/// that proves the server is actually up before we exercise shutdown.
fn complete_initialize_handshake(child: &mut Child) {
    let mut stdin = child.stdin.take().expect("child stdin must be piped");
    let stdout = child.stdout.take().expect("child stdout must be piped");
    let reader = BufReader::new(stdout);

    writeln!(stdin, "{}", initialize_request(1)).expect("write initialize to child stdin");
    stdin.flush().expect("flush initialize request");

    let response = read_line_with_timeout(reader, SHUTDOWN_TIMEOUT)
        .unwrap_or_else(|| panic!("no initialize response within {SHUTDOWN_TIMEOUT:?}"));
    assert!(
        response.contains("\"protocolVersion\""),
        "expected an initialize response, got: {response}"
    );

    // Put stdin back so the caller controls exactly when EOF happens; the
    // stdout reader (and its handshake-reading thread) is dropped — we
    // don't need any more output for this test.
    child.stdin = Some(stdin);
}

/// Poll `Child::try_wait` until the process exits or `timeout` elapses.
/// Returns the exit status, or `None` on timeout (process still alive).
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

#[test]
fn server_exits_when_stdin_reaches_eof() {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");
    let mut child = spawn_server(store_dir.path());

    complete_initialize_handshake(&mut child);

    // Close stdin: this is exactly what happens when an MCP client
    // disconnects (a health-check like `claude mcp list`, or a closed
    // Claude Code session) — the transport must observe EOF and shut down.
    drop(child.stdin.take());

    let status = wait_for_exit(&mut child, SHUTDOWN_TIMEOUT);

    if status.is_none() {
        // Don't leak the hung process into the rest of the test run.
        let _ = child.kill();
        let _ = child.wait();
    }

    let status = status.unwrap_or_else(|| {
        panic!(
            "server did not exit within {SHUTDOWN_TIMEOUT:?} of stdin EOF — \
             it is not observing transport closure (#1448)"
        )
    });
    assert!(
        status.success(),
        "server exited non-zero on stdin EOF: {status:?}"
    );
}

#[test]
fn store_lock_is_released_after_stdin_eof_so_a_second_session_can_connect() {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");

    let mut first = spawn_server(store_dir.path());
    complete_initialize_handshake(&mut first);
    drop(first.stdin.take());
    let first_status = wait_for_exit(&mut first, SHUTDOWN_TIMEOUT);
    if first_status.is_none() {
        let _ = first.kill();
        let _ = first.wait();
    }
    assert!(
        first_status.is_some(),
        "first server did not exit within {SHUTDOWN_TIMEOUT:?} of stdin EOF (#1448); \
         a second session on the same store cannot be expected to connect"
    );

    // The regression this second half catches: even if the first process
    // eventually exits, a still-held OS-level lock (released too late, or
    // never, by a lingering background task) would make this second
    // `initialize` hang or fail with `Storage(DatabaseLocked)` — exactly the
    // "opaque Failed to connect" symptom reported in #1448.
    let mut second = spawn_server(store_dir.path());
    let mut stdin = second
        .stdin
        .take()
        .expect("second child stdin must be piped");
    let stdout = second
        .stdout
        .take()
        .expect("second child stdout must be piped");
    let reader = BufReader::new(stdout);

    writeln!(stdin, "{}", initialize_request(1)).expect("write initialize to second child");
    stdin
        .flush()
        .expect("flush initialize request to second child");

    let response = read_line_with_timeout(reader, SHUTDOWN_TIMEOUT).unwrap_or_else(|| {
        let _ = second.kill();
        let _ = second.wait();
        panic!(
            "second server on the same store did not answer initialize within \
             {SHUTDOWN_TIMEOUT:?} — the store lock from the first (EOF-closed) \
             session was not released (#1448)"
        );
    });
    assert!(
        response.contains("\"protocolVersion\""),
        "expected an initialize response from the second session, got: {response}"
    );

    drop(stdin);
    let second_status = wait_for_exit(&mut second, SHUTDOWN_TIMEOUT);
    if second_status.is_none() {
        let _ = second.kill();
        let _ = second.wait();
    }
}
