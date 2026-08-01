//! Process-level proof that the daemon dispatches on the backend NAME the
//! library handed it (#1751, lot 3).
//!
//! Why a spawned process rather than a unit test: the defect being closed was
//! not in the selector — `select_extractor` has carried the backend's name
//! since #1734 — it was in `main.rs` throwing that name away:
//!
//! ```ignore
//! ExtractorSelection::NeedsRemoteConfig(_) => build_ollama_extractor()
//! ```
//!
//! An in-process test of the selector stays green against that line. Only
//! running the real binary shows which client it actually built.
//!
//! **How each test tells the two apart, without any server running.** The two
//! backends disagree about what a URL default means: Ollama has one canonical
//! local address and defaults to it, while `openai` names a protocol a dozen
//! servers speak and therefore refuses to guess. So the startup refusal names
//! `..._URL` for one and only `..._MODEL` for the other — a difference that
//! exists only if the dispatch read the name. The pair is the point: either
//! test alone would pass on a daemon hard-wired to the backend it happens to
//! assert.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

#![cfg(all(feature = "extract", feature = "ollama"))]

use std::process::{Command, Output};

/// Generous ceiling for "the daemon refused its configuration and exited".
/// This path opens the store and then fails before serving anything, so it is
/// bounded by process startup, not by any network call.
const STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Spawn the real binary with `VELESDB_MEMORY_EXTRACTOR=<backend>` on a
/// throwaway store, and return what it did.
///
/// Every other `VELESDB_MEMORY_*` variable is cleared: the developer running
/// this suite may well have a real daemon configured, and inheriting their
/// `VELESDB_MEMORY_EXTRACTOR_MODEL` would make the refusal under test vanish.
fn start_with_extractor(backend: &str) -> Output {
    let store = tempfile::tempdir().expect("tempdir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
        .env_clear()
        // `HOME` survives because the config-file lookup and the default store
        // path both read it; pointing it at the throwaway directory keeps this
        // test off the developer's own `~/.velesdb-memory`.
        .env("HOME", store.path())
        .env("VELESDB_MEMORY_PATH", store.path())
        .env("VELESDB_MEMORY_EXTRACTOR", backend)
        .env("VELESDB_MEMORY_QUIET", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn velesdb-memory");

    let start = std::time::Instant::now();
    loop {
        if let Some(_status) = child.try_wait().expect("wait on velesdb-memory") {
            return child.wait_with_output().expect("collect output");
        }
        assert!(
            start.elapsed() < STARTUP_TIMEOUT,
            "velesdb-memory did not exit within {STARTUP_TIMEOUT:?} — it should \
             have refused its extractor configuration at startup"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn openai_refuses_without_a_url_because_it_has_no_default_server() {
    let output = start_with_extractor("openai");
    let stderr = stderr_of(&output);
    assert!(
        !output.status.success(),
        "a backend missing its required configuration must not start; got {output:?}"
    );
    assert!(
        stderr.contains("VELESDB_MEMORY_EXTRACTOR_URL"),
        "the refusal must name the variable to set. An `openai` request served \
         by the Ollama builder would have defaulted the URL and never mentioned \
         it — which is precisely the bug. Got: {stderr}"
    );
}

#[test]
fn ollama_still_defaults_its_url_and_asks_only_for_the_model() {
    // The other half of the pair, and the compatibility guarantee: routing by
    // name must not have changed what `ollama` does.
    let output = start_with_extractor("ollama");
    let stderr = stderr_of(&output);
    assert!(!output.status.success(), "no model configured, so no start");
    assert!(
        stderr.contains("VELESDB_MEMORY_EXTRACTOR_MODEL"),
        "Ollama's own refusal is about the model, got: {stderr}"
    );
    assert!(
        !stderr.contains("VELESDB_MEMORY_EXTRACTOR_URL"),
        "Ollama defaults its URL to the canonical local address and must keep \
         doing so — naming the URL here would mean the `openai` builder answered \
         for it. Got: {stderr}"
    );
}

#[test]
fn an_unknown_backend_is_refused_before_any_client_is_built() {
    let output = start_with_extractor("lmstudio");
    let stderr = stderr_of(&output);
    assert!(!output.status.success(), "an unknown name must not start");
    assert!(
        stderr.contains("lmstudio") && stderr.contains("openai"),
        "the refusal must quote what was asked for and name the accepted forms, \
         got: {stderr}"
    );
}

/// Spawn the binary configured for the EMBEDDING role, with `extra` variables
/// on top. Unlike [`start_with_extractor`] this leaves `VELESDB_MEMORY_QUIET`
/// unset, because some of what is asserted here is written to stderr at
/// startup.
fn start_with_embedder(backend: &str, extra: &[(&str, &str)]) -> Output {
    let store = tempfile::tempdir().expect("tempdir");
    let mut command = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"));
    command
        .env_clear()
        .env("HOME", store.path())
        .env("VELESDB_MEMORY_PATH", store.path())
        .env("VELESDB_MEMORY_EMBEDDER", backend);
    for (name, value) in extra {
        command.env(name, value);
    }
    command
        .stdin(std::process::Stdio::null())
        .output()
        .expect("failed to spawn velesdb-memory")
}

#[test]
fn the_embedding_role_refuses_openai_without_a_url_too() {
    // The symmetry is the contract: an operator who has configured one role
    // has configured the other. This role had no second backend to get wrong
    // before #1751, which is exactly why its `_` was the easier one to leave.
    let output = start_with_embedder("openai", &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "no URL configured, so no start");
    assert!(
        stderr.contains("VELESDB_MEMORY_EMBEDDER_URL"),
        "the refusal must name the embedding role's own variable, got: {stderr}"
    );
}

#[test]
fn the_role_named_url_wins_over_the_legacy_alias_and_says_so_once() {
    // Port 1 and port 2 both refuse instantly, so this needs no server: what
    // matters is WHICH address the daemon tried, which the connection failure
    // names verbatim.
    let output = start_with_embedder(
        "openai",
        &[
            ("VELESDB_MEMORY_EMBEDDER_URL", "http://127.0.0.1:1"),
            ("VELESDB_MEMORY_OLLAMA_URL", "http://127.0.0.1:2"),
            ("VELESDB_MEMORY_EMBEDDER_MODEL", "bge-m3"),
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("127.0.0.1:1"),
        "the role-named URL must be the one actually called, got: {stderr}"
    );
    assert!(
        !stderr.contains("127.0.0.1:2"),
        "the legacy alias must not be what was called, got: {stderr}"
    );
    let notices = stderr
        .lines()
        .filter(|line| line.contains("VELESDB_MEMORY_OLLAMA_URL"))
        .count();
    assert_eq!(
        notices, 1,
        "exactly ONE notice about the disagreement — not one per read, not none. \
         Got: {stderr}"
    );
}

#[test]
fn the_legacy_alias_alone_still_configures_the_embedder() {
    // The compatibility guarantee, stated as a test: a setup that predates the
    // role-named variables must keep working with nothing changed.
    let output = start_with_embedder(
        "openai",
        &[
            ("VELESDB_MEMORY_OLLAMA_URL", "http://127.0.0.1:2"),
            ("VELESDB_MEMORY_OLLAMA_MODEL", "bge-m3"),
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("127.0.0.1:2"),
        "the legacy alias must still be honoured when it is the only one set, \
         got: {stderr}"
    );
    assert!(
        !stderr.contains("VELESDB_MEMORY_OLLAMA_URL is set under two names"),
        "using the alias as intended is not a conflict, got: {stderr}"
    );
}

#[test]
fn an_empty_api_token_is_refused_rather_than_sent_as_a_blank_bearer() {
    // Edge case with teeth: `VELESDB_MEMORY_EXTRACTOR_API_TOKEN=` is what a
    // shell produces when an expansion yields nothing. Sending
    // `Authorization: Bearer ` gets rejected as a BAD credential, sending
    // nothing gets rejected as a MISSING one, and the operator debugs the
    // wrong half of their setup either way.
    let store = tempfile::tempdir().expect("tempdir");
    let output = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
        .env_clear()
        .env("HOME", store.path())
        .env("VELESDB_MEMORY_PATH", store.path())
        .env("VELESDB_MEMORY_EXTRACTOR", "openai")
        .env("VELESDB_MEMORY_EXTRACTOR_URL", "http://localhost:8028")
        .env("VELESDB_MEMORY_EXTRACTOR_MODEL", "ornith")
        .env("VELESDB_MEMORY_EXTRACTOR_API_TOKEN", "")
        .env("VELESDB_MEMORY_QUIET", "1")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("failed to spawn velesdb-memory");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "an empty token must not start");
    assert!(
        stderr.contains("VELESDB_MEMORY_EXTRACTOR_API_TOKEN"),
        "the refusal must name the variable, got: {stderr}"
    );
}
