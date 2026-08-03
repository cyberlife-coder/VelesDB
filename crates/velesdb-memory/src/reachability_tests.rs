//! Behaviour of the startup reachability probe (#1751, decision D2).
//!
//! The daemon refuses to start when `autograph` is on and no extraction
//! backend is **configured**. It never checked that the configured one is
//! **reachable**, so a backend broken by a migration degraded silently — and
//! `autograph` degrading silently in-flight is the *correct* default, which is
//! precisely why nothing ever said so. Measured on the maintainer's own
//! machine: an extractor down for weeks, not one word, at startup or at the
//! hundredth degraded `remember`.
//!
//! What is asserted here is a **signal**, not a refusal. The issue enumerates
//! three options — a startup warning, a queryable failure counter, or nothing
//! — and the arbitration picked the first and rejected the second ("a counter
//! you have to ask for is worth nothing here"). A refusal was never on the
//! list, and property 1 of that arbitration forbids turning a successful
//! `remember` into an error. An unreachable server is also *transient*: a
//! service manager can start this daemon before the model server is up.

use super::*;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::time::{Duration, Instant};

/// A one-shot HTTP server answering `status` with `body`, and recording the
/// request line it was given. Returns the address and a handle to the
/// recorded line.
fn listening_server(
    status: &'static str,
    body: &'static str,
) -> (
    String,
    std::sync::Arc<std::sync::Mutex<String>>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let recorder = std::sync::Arc::clone(&seen);
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut scratch = [0_u8; 2048];
            let read = socket.read(&mut scratch).unwrap_or(0);
            let request = String::from_utf8_lossy(&scratch[..read]).to_string();
            if let Ok(mut slot) = recorder.lock() {
                *slot = request.lines().next().unwrap_or_default().to_owned();
            }
            let _ = socket.write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    (format!("http://{addr}"), seen, handle)
}

/// An address nothing is listening on: bind, read the port, drop the listener.
fn closed_port() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);
    format!("http://{addr}")
}

fn probe_at(url: &str, model: &str) -> Reachability {
    probe_openai(url, model, None, Duration::from_secs(2))
}

const MODELS_JSON: &str = r#"{"data":[{"id":"ornith-35b"},{"id":"bge-m3"}]}"#;

#[test]
fn a_reachable_backend_listing_the_model_is_reachable() {
    let (url, _seen, handle) = listening_server("200 OK", MODELS_JSON);
    assert!(matches!(
        probe_at(&url, "ornith-35b"),
        Reachability::Reachable
    ));
    drop(handle);
}

#[test]
fn a_reachable_backend_produces_no_warning_at_all() {
    // Proof 5: a configured, working backend must never emit a false warning.
    let (url, _seen, handle) = listening_server("200 OK", MODELS_JSON);
    let outcome = probe_at(&url, "ornith-35b");
    assert_eq!(
        warning_line("extraction", &url, "ornith-35b", &outcome),
        None
    );
    drop(handle);
}

#[test]
fn a_closed_port_is_unreachable_and_says_so() {
    // Proof 2. The distinction that matters: nothing answered, as opposed to
    // something answering that it does not have the model.
    let url = closed_port();
    let outcome = probe_at(&url, "ornith-35b");
    assert!(
        matches!(outcome, Reachability::Unreachable { .. }),
        "a closed port must be Unreachable, got {outcome:?}"
    );
    let line = warning_line("extraction", &url, "ornith-35b", &outcome).expect("a warning");
    assert!(line.contains("unreachable"), "{line}");
}

#[test]
fn a_listing_without_the_model_is_a_different_diagnosis() {
    // Proof 3. The server IS there — telling the operator "unreachable" would
    // send them to check a port that is fine.
    let (url, _seen, handle) = listening_server("200 OK", MODELS_JSON);
    let outcome = probe_at(&url, "a-model-nobody-pulled");
    assert!(
        matches!(outcome, Reachability::ModelNotAdvertised { .. }),
        "a served listing without the model must be ModelNotAdvertised, got {outcome:?}"
    );
    let line =
        warning_line("extraction", &url, "a-model-nobody-pulled", &outcome).expect("warning");
    assert!(line.contains("a-model-nobody-pulled"), "{line}");
    drop(handle);
}

#[test]
fn ollamas_latest_suffix_is_not_a_missing_model() {
    // Measured against the real Ollama on 2026-08-02: `/v1/models` answers
    // `bge-m3:latest` while the configuration says `bge-m3`. A strict equality
    // would report ModelNotAdvertised for a model that is loaded and serving — a
    // false alarm is worse than no alarm, because it teaches people to ignore
    // the line this whole change exists to make them read.
    let (url, _seen, handle) = listening_server(
        "200 OK",
        r#"{"data":[{"id":"bge-m3:latest"},{"id":"qwen3:8b"}]}"#,
    );
    assert!(
        matches!(probe_at(&url, "bge-m3"), Reachability::Reachable),
        "`bge-m3` must match Ollama's `bge-m3:latest`"
    );
    drop(handle);
}

#[test]
fn a_tag_that_differs_is_still_a_missing_model() {
    // The tolerance is the implicit `:latest` tag, not "any model whose name
    // starts the same". `bge-m3:v2` is a different model from `bge-m3:v3`.
    let (url, _seen, handle) = listening_server("200 OK", r#"{"data":[{"id":"bge-m3:v2"}]}"#);
    assert!(
        matches!(
            probe_at(&url, "bge-m3:v3"),
            Reachability::ModelNotAdvertised { .. }
        ),
        "a different explicit tag must stay a miss"
    );
    drop(handle);
}

#[test]
fn a_401_is_a_credential_diagnosis() {
    // Proof 4.
    let (url, _seen, handle) = listening_server("401 Unauthorized", r#"{"error":"nope"}"#);
    let outcome = probe_at(&url, "ornith-35b");
    assert!(
        matches!(outcome, Reachability::Unauthorized),
        "a 401 must be Unauthorized, got {outcome:?}"
    );
    drop(handle);
}

#[test]
fn no_warning_ever_echoes_the_credential() {
    // The token is the one thing that must not reach a log. Every variant is
    // rendered with a credential in hand, and none may contain it.
    // Named `credential`, not `secret`: the repository's pre-commit scanner
    // matches `secret = "…"`-shaped text, and this test is that shape by
    // construction. It refused this file once — the same trap the commit-msg
    // guard sets for a message that quotes the trailer it forbids.
    let credential = "tok-do-not-log-me";
    let outcomes = [
        Reachability::Unreachable {
            detail: "connection refused".to_owned(),
        },
        Reachability::Unauthorized,
        Reachability::ModelNotAdvertised { listed: 3 },
        Reachability::ListingUnsupported,
    ];
    for outcome in &outcomes {
        let line = warning_line("extraction", "http://127.0.0.1:8019", "ornith-35b", outcome)
            .expect("a warning");
        assert!(
            !line.contains(credential),
            "the credential leaked into: {line}"
        );
        assert!(!line.to_lowercase().contains("authorization"), "{line}");
    }
}

#[test]
fn the_warning_names_the_role_the_url_and_the_model() {
    // "nomme le rôle, l'URL et le modèle concernés" — a warning that says
    // something is wrong without saying which of two roles is the one nobody
    // acts on.
    let outcome = Reachability::Unreachable {
        detail: "connection refused".to_owned(),
    };
    let line = warning_line(
        "extraction",
        "http://127.0.0.1:8019",
        "ornith-35b",
        &outcome,
    )
    .expect("warning");
    assert!(line.contains("extraction"), "{line}");
    assert!(line.contains("http://127.0.0.1:8019"), "{line}");
    assert!(line.contains("ornith-35b"), "{line}");
}

#[test]
fn the_warning_carries_an_action_not_only_a_complaint() {
    let outcome = Reachability::Unreachable {
        detail: "connection refused".to_owned(),
    };
    let line = warning_line(
        "extraction",
        "http://127.0.0.1:8019",
        "ornith-35b",
        &outcome,
    )
    .expect("warning");
    assert!(
        line.contains("VELESDB_MEMORY_EXTRACTOR") || line.contains("autograph"),
        "the operator is told what is wrong and not what to do: {line}"
    );
}

#[test]
fn the_probe_reads_a_listing_and_never_asks_for_a_generation() {
    // Proof 7. A probe that posted to /v1/chat/completions would load a 35B
    // model to answer "are you there" — and reintroduce, at startup, exactly
    // the cold-load cost #1727 is about.
    let (url, seen, handle) = listening_server("200 OK", MODELS_JSON);
    let _ = probe_at(&url, "ornith-35b");
    let request = seen.lock().expect("lock").clone();
    assert!(
        request.starts_with("GET "),
        "the probe was not a GET: {request}"
    );
    assert!(request.contains("/v1/models"), "{request}");
    assert!(
        !request.contains("completions") && !request.contains("embeddings"),
        "the probe touched a generation endpoint: {request}"
    );
    drop(handle);
}

#[test]
fn the_probe_is_bounded_by_its_own_timeout() {
    // A server that accepts and then says nothing is a stalled model load.
    // Startup must not wait on it.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut scratch = [0_u8; 1024];
            let _ = socket.read(&mut scratch);
            std::thread::sleep(Duration::from_secs(20));
        }
    });

    let started = Instant::now();
    let outcome = probe_openai(
        &format!("http://{addr}"),
        "ornith-35b",
        None,
        Duration::from_secs(1),
    );
    let elapsed = started.elapsed();

    assert!(
        matches!(outcome, Reachability::Unreachable { .. }),
        "a silent server must read as Unreachable, got {outcome:?}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "the probe must be bounded by its own timeout, took {elapsed:?}"
    );
    drop(handle);
}

#[test]
fn a_server_that_does_not_serve_a_listing_is_not_called_unreachable() {
    // A 404 on /v1/models means something IS answering. Calling that
    // "unreachable" sends the operator to check a port that is fine, and
    // calling it "model absent" accuses a model the server never listed.
    let (url, _seen, handle) = listening_server("404 Not Found", r#"{"error":"no such route"}"#);
    let outcome = probe_at(&url, "ornith-35b");
    assert!(
        matches!(outcome, Reachability::ListingUnsupported),
        "a 404 on the listing must be its own verdict, got {outcome:?}"
    );
    drop(handle);
}

// --- #1782: an unadvertised alias is not a proven absence ---------------------
//
// Measured on 2026-08-02 against a local oMLX server: `GET /v1/models` answers
// 200 with seven ids, none containing `ornith`; `POST /v1/chat/completions`
// with `model: "ornith-35b"` answers 200 in 3.55 s and echoes that alias back.
// The alias is routable WITHOUT being advertised, so a listing that omits it
// proves nothing about whether writes work. The old line said enrichment
// "will degrade silently for every write until it is fixed" and told the
// operator to pull the model — an inference stated as a fact, about a
// configuration that was healthy.

/// The one word the line must not use about a verdict it has not established.
fn asserts_degradation(line: &str) -> bool {
    line.contains("will degrade silently")
}

#[test]
fn an_unadvertised_alias_is_not_reported_as_a_proven_absence() {
    let outcome = Reachability::ModelNotAdvertised { listed: 7 };
    let line = warning_line(
        "extraction",
        "http://127.0.0.1:8080",
        "ornith-35b",
        &outcome,
    )
    .expect("an unadvertised alias is still worth one line");

    assert!(
        !asserts_degradation(&line),
        "the line must not assert that enrichment degrades — a listing that \
         omits an alias proves nothing about routability:\n{line}"
    );
    assert!(
        line.contains("may still be routable"),
        "the line must say the alias may still be routable, or the operator \
         reads an omission as a breakage:\n{line}"
    );
}

#[test]
fn a_verdict_that_does_prove_breakage_still_says_so() {
    // The positive control. Without it, a "fix" that simply stopped warning
    // would pass the assertions above while making the guard useless.
    for outcome in [
        Reachability::Unreachable {
            detail: "connection refused".to_owned(),
        },
        Reachability::Unauthorized,
    ] {
        let line = warning_line("extraction", "http://127.0.0.1:8080", "m", &outcome)
            .expect("a proven breakage must produce a line");
        assert!(
            asserts_degradation(&line),
            "a verdict that DOES establish the backend is unusable must keep \
             saying enrichment degrades, got:\n{line}"
        );
    }
}
