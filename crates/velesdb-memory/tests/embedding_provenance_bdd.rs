//! End-to-end behaviour of the embedding-model record (#1751, arbitration A1).
//!
//! The unit tests next to `src/embedding_provenance.rs` prove the comparison;
//! these prove the daemon actually runs it, and — the part no pure test can
//! reach — that the record is written for a store with no facts and **never**
//! over one that has them.
//!
//! Why the same-dimension case is the one that matters: `velesdb-core` already
//! refuses a collection whose dimension differs, so `bge-m3` (1024) against
//! `all-minilm` (384) was never the silent failure. Two *different* models of
//! the *same* width open fine and return nonsense — this crate's own `hash`
//! embedder is 384-dimensional, and so is `all-minilm`. That is the gap under
//! test here.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

#![cfg(feature = "persistence")]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};

/// The daemon opens the store, answers, and exits on EOF; nothing here waits
/// on a network call.
const EXIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const PROVENANCE_FILE: &str = "embedding-provenance.json";

/// Run the daemon against `store`, optionally sending `requests` over stdio,
/// then close stdin and return `(exit_ok, stderr)`.
///
/// `env_clear` for the same reason as the dispatch suite: a developer running
/// this with a real `VELESDB_MEMORY_EMBEDDER` exported would otherwise test
/// their own configuration instead of the default one.
fn run_daemon(store: &Path, requests: &[String]) -> (bool, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
        .env_clear()
        .env("HOME", store)
        .env("VELESDB_MEMORY_PATH", store)
        .env("VELESDB_MEMORY_QUIET", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn velesdb-memory");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
        for request in requests {
            writeln!(stdin, "{request}").expect("write request");
            stdin.flush().expect("flush");
            // One reply per request, read before sending the next: the server
            // answers concurrently, so a batched write lets a later reply
            // overtake an earlier one.
            if request.contains("\"id\"") {
                let mut line = String::new();
                stdout.read_line(&mut line).expect("read reply");
                // BOTH failure shapes. A refused tool call comes back as a
                // perfectly successful JSON-RPC *result* carrying
                // `isError: true`, so checking only for a JSON-RPC `error`
                // reads a failed `remember` as a stored fact — which is how
                // this very harness first "proved" that a seeded store had
                // nothing in it.
                assert!(
                    !line.contains("\"error\"") && !line.contains("\"isError\":true"),
                    "the daemon refused a request: {line}"
                );
            }
        }
    }
    child.stdin.take();

    let start = std::time::Instant::now();
    loop {
        if child.try_wait().expect("wait").is_some() {
            let output = child.wait_with_output().expect("collect");
            return (
                output.status.success(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
            );
        }
        assert!(
            start.elapsed() < EXIT_TIMEOUT,
            "the daemon did not exit within {EXIT_TIMEOUT:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn initialize() -> String {
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"provenance_bdd","version":"0"}}}"#.to_owned()
}

fn initialized() -> String {
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned()
}

/// The argument is `fact`, not `content` — `content` belongs to the context
/// compiler's fragments, and passing it here returns `isError: true` inside an
/// otherwise successful frame.
fn remember(text: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"remember","arguments":{{"fact":"{text}"}}}}}}"#
    )
}

fn record_of(store: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(store.join(PROVENANCE_FILE)).ok()?;
    Some(serde_json::from_str(&raw).expect("the record must be valid JSON"))
}

#[test]
fn a_fresh_store_records_the_model_it_was_created_with() {
    let store = tempfile::tempdir().expect("tempdir");
    let (ok, stderr) = run_daemon(store.path(), &[initialize()]);
    assert!(ok, "a fresh store must open cleanly; stderr: {stderr}");

    let record = record_of(store.path()).expect("a fresh store must be stamped");
    assert_eq!(record["model"], "hash", "the default embedder is `hash`");
    assert_eq!(
        record["dimension"], 384,
        "and `hash` is 384-dimensional — the same width as `all-minilm`, which \
         is exactly why the model has to be recorded alongside it"
    );
    assert!(
        record.get("backend").is_none(),
        "the backend is a transport and must not be recorded: the same model \
         over Ollama or over an OpenAI-compatible API yields the same vectors"
    );
}

#[test]
fn a_recorded_model_is_refused_by_a_different_one_of_the_same_width() {
    // THE case the record exists for. 384 on both sides, so `velesdb-core`'s
    // dimension check passes and would have let this open silently.
    let store = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        store.path().join(PROVENANCE_FILE),
        r#"{"model":"all-minilm","dimension":384}"#,
    )
    .expect("pre-stamp the store");

    let (ok, stderr) = run_daemon(store.path(), &[]);
    assert!(!ok, "a model change must not open the store");
    assert!(
        stderr.contains("all-minilm") && stderr.contains("hash"),
        "the refusal must name both configurations, got: {stderr}"
    );
}

#[test]
fn a_store_that_already_holds_facts_is_never_stamped_retroactively() {
    // A store created before this record existed. Simulated the only honest
    // way: let the daemon create one, put a fact in it, then remove the record
    // so the next open sees data with no provenance.
    let store = tempfile::tempdir().expect("tempdir");
    let (ok, stderr) = run_daemon(
        store.path(),
        &[initialize(), initialized(), remember("un fait durable")],
    );
    assert!(ok, "the seeding run must succeed; stderr: {stderr}");
    std::fs::remove_file(store.path().join(PROVENANCE_FILE)).expect("remove the record");

    let (ok, stderr) = run_daemon(store.path(), &[initialize()]);
    assert!(
        ok,
        "a store with no record must still open — the compatibility guarantee; \
         stderr: {stderr}"
    );
    assert!(
        record_of(store.path()).is_none(),
        "stamping a store that already holds vectors would carve a provenance \
         nobody verified, and every later check would trust it"
    );
}

#[test]
fn an_empty_store_with_no_record_is_stamped_on_the_next_open() {
    // The edge that makes the rule "no facts", not "new directory": a store
    // that exists but holds nothing has no vector that could have come from
    // another model, so recording states something true. A directory-shaped
    // test would also be defeated by the config file the docs put right here.
    let store = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        store.path().join("velesdb-memory.toml"),
        "# a config file living beside the store, exactly as documented\n",
    )
    .expect("write config");

    let (ok, stderr) = run_daemon(store.path(), &[initialize()]);
    assert!(ok, "must open; stderr: {stderr}");
    assert!(
        record_of(store.path()).is_some(),
        "a config file sitting in the store directory must not be mistaken for \
         stored data"
    );
}

#[test]
fn a_damaged_record_stops_the_daemon_instead_of_silently_skipping_the_check() {
    let store = tempfile::tempdir().expect("tempdir");
    std::fs::write(store.path().join(PROVENANCE_FILE), "{ truncated").expect("write junk");

    let (ok, stderr) = run_daemon(store.path(), &[]);
    assert!(!ok, "an unreadable record must not be treated as absent");
    assert!(
        stderr.contains(PROVENANCE_FILE),
        "the refusal must name the file to delete, got: {stderr}"
    );
}
