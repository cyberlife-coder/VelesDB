//! Two OS processes saving working-contexts on the same store lose no index
//! entries (#1958) — because they never run concurrently in the first place.
//!
//! #1958 asked for a compare-and-swap on the storage trait so the working-
//! context index's read-modify-write could not race across processes. The
//! premise was the index lock's own doc-comment, which claimed "two
//! processes opening the same store still race". That claim was stale:
//! `velesdb-core`'s `Database::open_impl` takes an exclusive `flock` on
//! `velesdb.lock` AT OPEN and holds it for the `Database`'s whole lifetime —
//! not per write. A second process fails at `open` with `DatabaseLocked`
//! before it can reach ANY read-modify-write, of the index or of anything
//! else. Cross-process index atomicity is therefore guaranteed by mutual
//! exclusion at the store boundary, and the intra-process mutex
//! (`WORKING_INDEX_WRITE`, proven by
//! `working_index_lock_contention_bdd.rs` and
//! `memory_bridge_tests.rs::test_concurrent_saves_on_one_project_keep_both_sessions_in_the_index`)
//! is the complete defense for the only concurrency that can exist.
//!
//! This test enacts the issue's success criterion with real processes:
//! daemon A saves sessions in a loop; daemon B tries to work on the same
//! store mid-loop and must fail fast at open (never writing anything);
//! A keeps saving unharmed; after A exits, a successor process C sees
//! EVERY session A saved — zero index entries lost, across contention
//! and across the process handoff.
//!
//! Harness notes: MCP-over-stdio plumbing follows
//! `online_migration_process.rs`'s `ProcessClient`; the contender-fails-fast
//! half mirrors `http_lock_contention.rs` (same store lock, stdio variant).
//! Every wait in here is bounded — including the request/response round
//! trips: the failure mode this test guards against is a daemon that stops
//! answering, which a bare `read_line` would turn into a suite hang instead
//! of a red test.

#![cfg(all(feature = "mcp", feature = "persistence"))]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Bound on the contender's fail-fast exit: generously above
/// `daemon_startup`'s bounded lock retry (three attempts separated by two
/// 500 ms pauses), far below a hang.
const CONTENDER_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Bound on any single request/response round trip (and on shutdown). These
/// are local-process saves against a hash embedder — sub-second in practice,
/// so ten seconds separates "slow" from "hung" unambiguously.
const STEP: Duration = Duration::from_secs(10);

/// The project every session is saved under — contention on the index is
/// per project, so one shared project is the interesting case.
const PROJECT: &str = "two-daemons";

/// Sessions daemon A saves before the contender attempts its open, and
/// after the contender has failed — proving the failed attempt disturbed
/// nothing on either side of it.
const SESSIONS_BEFORE: [&str; 3] = ["a1", "a2", "a3"];
const SESSIONS_AFTER: [&str; 3] = ["a4", "a5", "a6"];

struct StdioDaemon {
    child: Child,
    stdin: Option<ChildStdin>,
    /// Response lines, fed by a dedicated reader thread — so every receive
    /// can carry a deadline. The thread ends on the daemon's stdout EOF,
    /// which drops the sender and turns further receives into clean errors.
    lines: mpsc::Receiver<String>,
    next_id: u64,
}

/// A panic anywhere mid-test (a failed assert) must not leave a live daemon
/// behind: it would outlive the `TempDir` it is writing into. Redundant
/// after a clean [`StdioDaemon::shutdown`] (both calls then fail, ignored).
impl Drop for StdioDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl StdioDaemon {
    fn spawn(store: &std::path::Path, home: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
            .env("HOME", home)
            .env("VELESDB_MEMORY_PATH", store)
            .env("VELESDB_MEMORY_EMBEDDER", "hash")
            .env("VELESDB_MEMORY_QUIET", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn velesdb-memory daemon");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let (line_tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut daemon = Self {
            child,
            stdin: Some(stdin),
            lines,
            next_id: 1,
        };
        daemon.initialize();
        daemon
    }

    fn initialize(&mut self) {
        self.request(
            "initialize",
            json!({
                "protocolVersion":"2024-11-05",
                "capabilities":{},
                "clientInfo":{"name":"working-index-two-daemons","version":"1"}
            }),
        );
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let mut params = json!({"name":name});
        params["arguments"] = arguments;
        let response = self.request("tools/call", params);
        assert_ne!(
            response["result"]["isError"], true,
            "tool {name}: {response}"
        );
        response["result"]["structuredContent"].clone()
    }

    fn save_session(&mut self, session: &str) {
        let saved = self.call(
            "save_working_context",
            json!({
                "project": PROJECT,
                "session": session,
                "working": {"goal": format!("goal of {session}")}
            }),
        );
        assert!(
            saved["id"].as_u64().is_some(),
            "save must return the stored fact id, got: {saved}"
        );
    }

    fn listed_sessions(&mut self) -> Vec<String> {
        let listed = self.call("list_working_contexts", json!({"project": PROJECT}));
        listed["sessions"]
            .as_array()
            .expect("sessions array")
            .iter()
            .map(|entry| entry["session"].as_str().expect("session name").to_owned())
            .collect()
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let mut frame = json!({"jsonrpc":"2.0","id":id,"method":method});
        frame["params"] = params;
        self.send(&frame);
        let line = self.lines.recv_timeout(STEP).unwrap_or_else(|_| {
            panic!("no response to {method} within {STEP:?} — the daemon is hung or dead")
        });
        let response: Value = serde_json::from_str(&line).expect("JSON response");
        assert_eq!(response["id"], id, "unexpected response: {response}");
        assert!(
            response.get("error").is_none(),
            "request failed: {response}"
        );
        response
    }

    fn send(&mut self, frame: &Value) {
        let stdin = self.stdin.as_mut().expect("live stdin");
        serde_json::to_writer(&mut *stdin, frame).expect("write frame");
        stdin.write_all(b"\n").expect("newline");
        stdin.flush().expect("flush");
    }

    /// EOF on stdin, then a clean exit — the lock-releasing shutdown
    /// `mcp_lifecycle.rs` proves. Bounded: a daemon that ignores EOF is a
    /// red test, not a hung suite.
    fn shutdown(mut self) {
        drop(self.stdin.take());
        let status = wait_for_exit(&mut self.child, STEP)
            .unwrap_or_else(|| panic!("daemon did not exit within {STEP:?} of stdin EOF"));
        assert!(status.success(), "daemon status: {status}");
    }
}

/// Poll `Child::try_wait` until exit or `timeout` — same bounded wait as
/// `http_lock_contention.rs`.
fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            // Surface the real cause instead of letting the caller blame a
            // timeout that never happened.
            Err(err) => panic!("try_wait failed: {err}"),
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Read all of `reader` on a helper thread, bounded — a direct
/// `read_to_string` on a hung child would hang the suite with it.
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
fn two_processes_saving_working_contexts_lose_no_index_entries() {
    let store_dir = tempfile::tempdir().expect("scratch store dir");
    let home_dir = tempfile::tempdir().expect("scratch home dir");

    // Given daemon A holding the store and saving sessions in a loop
    let mut daemon_a = StdioDaemon::spawn(store_dir.path(), home_dir.path());
    for session in SESSIONS_BEFORE {
        daemon_a.save_session(session);
    }

    // When daemon B tries to open the same store mid-loop, it fails at
    // open — before it can reach any read-modify-write of the index — with
    // the same actionable lock message the other transports print.
    let mut contender = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
        .env("HOME", home_dir.path())
        .env("VELESDB_MEMORY_PATH", store_dir.path())
        .env("VELESDB_MEMORY_EMBEDDER", "hash")
        .env("VELESDB_MEMORY_QUIET", "1")
        .stdin(Stdio::piped()) // held open: B must die from the lock, not EOF
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn contender daemon");
    let contender_stderr = contender.stderr.take().expect("contender stderr");
    let status = wait_for_exit(&mut contender, CONTENDER_EXIT_TIMEOUT).unwrap_or_else(|| {
        let _ = contender.kill();
        let _ = contender.wait();
        panic!(
            "a second daemon on an already-held store did not exit within \
             {CONTENDER_EXIT_TIMEOUT:?} — it must fail fast at open, not run \
             alongside the holder"
        );
    });
    assert!(
        !status.success(),
        "a second daemon on an already-held store must exit non-zero, got: {status:?}"
    );
    let stderr_text = drain_with_timeout(contender_stderr, Duration::from_secs(2)).to_lowercase();
    assert!(
        stderr_text.contains("velesdb_memory_path") && stderr_text.contains("pkill"),
        "expected the actionable lock-contention guidance, got: {stderr_text:?}"
    );

    // And A keeps saving, unharmed by the failed contender
    for session in SESSIONS_AFTER {
        daemon_a.save_session(session);
    }

    // Then A's index lists every session it saved...
    let seen_by_a = daemon_a.listed_sessions();
    for session in SESSIONS_BEFORE.iter().chain(&SESSIONS_AFTER) {
        assert!(
            seen_by_a.iter().any(|s| s == session),
            "session {session} missing from the index while A still runs: {seen_by_a:?}"
        );
    }
    daemon_a.shutdown();

    // ...and so does successor process C after the handoff: zero index
    // entries lost across the whole two-process scenario.
    let mut daemon_c = StdioDaemon::spawn(store_dir.path(), home_dir.path());
    let seen_by_c = daemon_c.listed_sessions();
    for session in SESSIONS_BEFORE.iter().chain(&SESSIONS_AFTER) {
        assert!(
            seen_by_c.iter().any(|s| s == session),
            "session {session} lost across the process handoff: {seen_by_c:?}"
        );
    }
    assert_eq!(
        seen_by_c.len(),
        SESSIONS_BEFORE.len() + SESSIONS_AFTER.len(),
        "the index must hold exactly the saved sessions: {seen_by_c:?}"
    );
    daemon_c.shutdown();
}
