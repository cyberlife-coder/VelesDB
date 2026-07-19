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

use std::io::{BufRead, BufReader, Read, Write};
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

/// Read everything available on `reader` until EOF (or `timeout` elapses,
/// whichever first) on a helper thread. Used to drain a dead process's
/// stderr — by the time this is called the process is already believed
/// gone, so `read_to_string` should hit EOF almost immediately; the timeout
/// is only a backstop against a stuck assumption.
fn drain_with_timeout<R: Read + Send + 'static>(mut reader: R, timeout: Duration) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = reader.read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    rx.recv_timeout(timeout).unwrap_or_default()
}

/// Upper bound the orphan test is willing to wait for the store lock to be
/// released. The watchdog is specified to poll on a ~2s cadence and the
/// intermediate process adds another ~1s grace period before it exits, so
/// in isolation this is already several polls of headroom over the
/// "≤ ~6s" self-exit target. Generous on top of that for the full suite
/// running several real `velesdb-memory` process spawns concurrently
/// (parallel `cargo test` threads), where OS scheduling/process-spawn
/// latency — not the watchdog logic itself — can dominate.
const ORPHAN_LOCK_RELEASE_TIMEOUT: Duration = Duration::from_secs(25);

/// Per-attempt bound for a single lock-release probe's `initialize`
/// round trip. Generous (matching [`SHUTDOWN_TIMEOUT`]) for the same
/// concurrent-process-spawn reason as [`ORPHAN_LOCK_RELEASE_TIMEOUT`]: a
/// tight per-probe timeout under a loaded test run flags "this probe was
/// slow to spawn" as "the lock is still held", which is a false signal —
/// the outer deadline is what actually bounds the regression check.
const PROBE_HANDSHAKE_TIMEOUT: Duration = SHUTDOWN_TIMEOUT;

#[test]
fn server_self_exits_when_orphaned_even_with_stdin_held_open() {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");
    let sync_dir = tempfile::tempdir().expect("create sync dir for handoff fifo");
    let fifo_path = sync_dir.path().join("orphan-handoff");
    let server_bin = env!("CARGO_BIN_EXE_velesdb-memory");

    let mkfifo_status = Command::new("mkfifo")
        .arg(&fifo_path)
        .status()
        .expect("failed to run mkfifo");
    assert!(
        mkfifo_status.success(),
        "mkfifo failed to create handoff fifo"
    );

    // Intermediate process: forks a subshell that `exec`s straight into the
    // real server binary — preserving the subshell's pid across the exec —
    // then blocks reading the handoff fifo before exiting. The server never
    // called `wait()` on anyone, and the outer shell never waits on it
    // either, so once the outer shell is gone the subshell (now the server)
    // reparents to init/launchd (an orphan).
    //
    // The fifo handoff (instead of a fixed `sleep`) is load-bearing, not
    // padding: OS reparenting can complete *faster than the freshly-exec'd
    // server binary finishes loading*, especially under a loaded test run
    // (several real process spawns competing for CPU/IO across parallel
    // `cargo test` threads). A wall-clock `sleep` can lose that race — the
    // server would then capture its "original" parent pid via `parent_id()`
    // only *after* it was already reparented, permanently defeating the
    // watchdog's change-detection (nothing to ever compare against). Blocking
    // the intermediate on the fifo until this test has proven — via a
    // completed `initialize` round trip — that the server is fully up
    // (necessarily past the point where `spawn_orphan_watchdog` captured its
    // real parent) makes the ordering deterministic instead of timing-based.
    //
    // `3<&0` duplicates the original stdin onto fd 3 *before* backgrounding:
    // POSIX/bash redirect an asynchronous (`&`) command's own fd 0 to
    // `/dev/null` when job control is off (always true for a non-interactive
    // `sh -c`), unless that command explicitly redirects fd 0 itself — which
    // `0<&3` inside the subshell does, re-attaching the real pipe. Without
    // this dance the exec'd server would see immediate stdin EOF from
    // `/dev/null` and exit via the already-covered EOF path instead of ever
    // becoming a live orphan.
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!(
            r#"exec 3<&0; (exec "{server_bin}" 0<&3 3<&-) & read -r _line < "{fifo}"; exit 0"#,
            fifo = fifo_path.display()
        ))
        .env("VELESDB_MEMORY_PATH", store_dir.path())
        .env("VELESDB_MEMORY_QUIET", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn intermediate shell");

    // Prove the (soon-to-be-orphaned) server actually came up before we
    // start asserting anything about its shutdown.
    complete_initialize_handshake(&mut child);

    // Unblock the intermediate's `read` now that the server has proven
    // (by answering `initialize`) that it already captured its real parent
    // pid — this is the deterministic handoff described above.
    std::fs::write(&fifo_path, b"go\n").expect("signal intermediate to exit via fifo");

    // Keep the write end of the stdin pipe open for the rest of the test —
    // this is the crucial plumbing detail: if this handle is dropped, the
    // server observes stdin EOF and exits via the *already-covered* path
    // (`server_exits_when_stdin_reaches_eof` above), which would make this
    // test pass for the wrong reason. Held open here, the server can only
    // exit because it detected its parent died, not because of EOF.
    let stdin_guard = child.stdin.take().expect("stdin must stay piped open");

    // Grab stderr now so we can inspect the orphan-shutdown log line once
    // the process is confirmed gone.
    let stderr = child.stderr.take().expect("stderr must be piped");

    // The regression signal: poll for the store lock being released by
    // trying to `initialize` a brand-new server on the same store path.
    // Today the orphaned server never notices its parent is gone (stdin is
    // still open, so it has no other shutdown trigger) and keeps holding
    // the `flock`, so every probe below fails/times out until the overall
    // deadline — reproducing the exact "next session can't connect"
    // symptom from #1448.
    let deadline = Instant::now() + ORPHAN_LOCK_RELEASE_TIMEOUT;
    let mut released = false;
    while Instant::now() < deadline {
        let mut probe = spawn_server(store_dir.path());
        let mut probe_stdin = probe.stdin.take().expect("probe stdin must be piped");
        let probe_stdout = probe.stdout.take().expect("probe stdout must be piped");
        let reader = BufReader::new(probe_stdout);

        let wrote = writeln!(probe_stdin, "{}", initialize_request(1)).is_ok()
            && probe_stdin.flush().is_ok();

        if wrote {
            if let Some(line) = read_line_with_timeout(reader, PROBE_HANDSHAKE_TIMEOUT) {
                if line.contains("\"protocolVersion\"") {
                    released = true;
                }
            }
        }

        drop(probe_stdin);
        let _ = probe.kill();
        let _ = probe.wait();

        if released {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    // Now that the lock is (supposedly) free, the orphaned server must
    // actually be gone — drop the stdin guard and drain whatever it logged.
    drop(stdin_guard);
    let stderr_text = drain_with_timeout(stderr, Duration::from_secs(2));

    assert!(
        released,
        "store lock was never released within {ORPHAN_LOCK_RELEASE_TIMEOUT:?} of the \
         server being orphaned with stdin still held open — the server does not \
         detect that its parent died and self-exit (#1448)"
    );
    assert!(
        stderr_text.to_lowercase().contains("parent"),
        "expected the orphaned server to log a clear parent-death shutdown \
         message on stderr, got: {stderr_text:?}"
    );
}

/// Regression this catches (#1448): a second `velesdb-memory` process opened
/// against a store another live process already holds must fail fast with
/// *actionable* guidance on stderr — not just the bare `Storage(DatabaseLocked
/// (..))` debug dump Rust's default `Result`-returning-`main` prints, which
/// gave users nothing to act on beyond an opaque "Failed to connect" in their
/// MCP client. It must also still exit non-zero (client health-checks depend
/// on that to report failure at all).
#[test]
fn database_locked_at_startup_prints_actionable_guidance_and_exits_nonzero() {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");

    // First server: opened and kept alive for the whole test — it
    // legitimately holds the store's single-writer flock throughout, playing
    // the role of the leaked/still-running process from #1448.
    let mut holder = spawn_server(store_dir.path());
    complete_initialize_handshake(&mut holder);

    // Second process on the same store path, while the first is still up.
    let mut contender = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
        .env("VELESDB_MEMORY_PATH", store_dir.path())
        .env("VELESDB_MEMORY_QUIET", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn contending velesdb-memory process");

    let stderr = contender
        .stderr
        .take()
        .expect("contender stderr must be piped");
    // The contender fails during synchronous startup, before it ever reads
    // the MCP transport — stdin/stdout are irrelevant to that failure, so
    // just let them close.
    drop(contender.stdin.take());
    drop(contender.stdout.take());

    let status = wait_for_exit(&mut contender, Duration::from_secs(5)).unwrap_or_else(|| {
        let _ = contender.kill();
        let _ = contender.wait();
        panic!(
            "a process opening an already-locked store did not exit within 5s \
             (#1448) — startup must fail fast (bounded retry), not hang"
        );
    });
    assert!(
        !status.success(),
        "a process opening an already-locked store must exit non-zero so \
         client health-checks can detect the failure, got: {status:?}"
    );

    let stderr_text = drain_with_timeout(stderr, Duration::from_secs(2));
    let lower = stderr_text.to_lowercase();
    assert!(
        lower.contains("velesdb_memory_path") && lower.contains("pkill"),
        "expected an actionable lock-contention message on stderr (naming \
         VELESDB_MEMORY_PATH as the escape hatch and pkill as the fix), \
         got: {stderr_text:?}"
    );

    let _ = holder.kill();
    let _ = holder.wait();
}
