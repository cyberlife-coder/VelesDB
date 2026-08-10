//! Tests for [`super::OllamaEmbedder`].
//!
//! Split out of `embedder.rs` to keep that file under the 500-NLOC gate,
//! following the crate's existing `#[path = "*_tests.rs"]` convention.

use super::*;

#[test]
fn request_body_carries_model_and_prompt() {
    let body = build_request_body("all-minilm", "hello world");
    let json: serde_json::Value = serde_json::from_str(&body).expect("valid json");
    assert_eq!(json["model"], "all-minilm");
    assert_eq!(json["prompt"], "hello world");
}

#[test]
fn request_body_pins_the_model_in_memory() {
    // Without `keep_alive` Ollama applies its own default and unloads the
    // model after a few idle minutes, so a call that follows a quiet spell
    // pays a full reload. Measured on this repo's own extraction model:
    // 14.19 s cold against 0.22 s warm — a 64x cliff an agent hits every
    // time it pauses to think.
    let body = build_request_body("all-minilm", "hello world");
    let json: serde_json::Value = serde_json::from_str(&body).expect("valid json");
    assert_eq!(
        json["keep_alive"], DEFAULT_KEEP_ALIVE,
        "request must pin the model for the daemon's lifetime"
    );
    assert!(
        json["keep_alive"].is_number(),
        "Ollama ignores a STRING \"-1\" and unloads after 5 minutes anyway"
    );
}

#[test]
fn parses_a_well_formed_embedding() {
    let vector = parse_embedding_response(r#"{"embedding":[0.1,0.2,0.3]}"#).expect("parse");
    assert_eq!(vector.len(), 3);
    assert!((vector[0] - 0.1_f32).abs() < f32::EPSILON);
}

#[test]
fn rejects_an_empty_embedding() {
    let parsed = parse_embedding_response(r#"{"embedding":[]}"#);
    assert!(matches!(parsed, Err(EmbedError::Empty)));
}

#[test]
fn rejects_a_malformed_response() {
    let parsed = parse_embedding_response(r#"{"oops":true}"#);
    assert!(matches!(parsed, Err(EmbedError::Backend(_))));
}

/// An Ollama that accepts the TCP connection and then never answers is
/// the failure this bound exists for: without it the embed call blocks
/// forever, and since `remember`/`save_working_context` embed before
/// writing, the MCP tool call hangs until the CLIENT times out — an
/// opaque transport error with nothing in the server's own error path.
/// Uses a 1 s agent so the test stays fast; the shipped ceiling is
/// `EMBED_TIMEOUT_SECS`.
#[test]
fn a_silent_ollama_is_bounded_instead_of_hanging_forever() {
    use std::io::Read as _;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    // Accept, read the request, then hold the socket open and answer
    // nothing at all — the exact shape of a stalled model load.
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut scratch = [0_u8; 1024];
            let _ = socket.read(&mut scratch);
            std::thread::sleep(Duration::from_secs(30));
        }
    });

    let agent = embed_agent(Duration::from_secs(1));
    let started = Instant::now();
    let outcome = request_embedding(&agent, &format!("http://{addr}"), "all-minilm", "hello");
    let elapsed = started.elapsed();

    assert!(
        matches!(outcome, Err(EmbedError::Backend(_))),
        "a silent backend must surface as a Backend error, got {outcome:?}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "the request must be bounded by the agent timeout, took {elapsed:?}"
    );
    drop(handle);
}

/// An Ollama that RESETS the connection is the failure actually observed in
/// the field: `/api/tags` answered in 7 ms, yet the embeddings POST died
/// with `Connection reset by peer (os error 54)`. A reset is not a timeout —
/// it fails instantly, so the 60 s ceiling buys nothing, and `ureq` refuses
/// to replay a POST with a body (`unit.rs`'s `is_retryable` demands an
/// idempotent method AND an empty body). The call therefore had exactly one
/// chance, on a pooled keep-alive connection the server had already closed.
///
/// The server here accepts, never reads, and closes: the kernel answers
/// unread bytes in the receive queue with an RST — the portable stand-in for
/// `SO_LINGER=0`, which would need a dependency this crate does not carry.
/// 1 initial attempt + the 2 replays of `HTTP_RETRIES`.
const EXPECTED_ATTEMPTS: usize = 3;

/// A listener that accepts `EXPECTED_ATTEMPTS` connections and closes each one
/// without reading it, so the kernel answers the unread request bytes with an
/// RST. Returns the address, a counter of accepted connections, and the thread
/// handle. The loop is bounded, so the thread ends on its own — no join needed
/// and no orphan left behind.
fn resetting_listener() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::thread::JoinHandle<()>,
) {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&attempts);
    let handle = std::thread::spawn(move || {
        for _ in 0..EXPECTED_ATTEMPTS {
            let Ok((socket, _)) = listener.accept() else {
                break;
            };
            seen.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            drop(socket);
        }
    });
    (addr, attempts, handle)
}

#[test]
fn an_ollama_that_resets_the_connection_is_retried_then_reported_actionably() {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    let (addr, attempts, handle) = resetting_listener();

    let agent = embed_agent(Duration::from_secs(2));
    let url = format!("http://{addr}");
    let started = Instant::now();
    let outcome = request_embedding(&agent, &url, "all-minilm", "hello");
    let elapsed = started.elapsed();

    assert_eq!(
        attempts.load(Ordering::SeqCst),
        EXPECTED_ATTEMPTS,
        "a reset connection must be replayed on a fresh connection, not \
         reported after a single doomed attempt"
    );

    let Err(EmbedError::Backend(message)) = outcome else {
        panic!("a reset backend must surface as a Backend error, got {outcome:?}");
    };
    for needle in [
        url.as_str(),
        "all-minilm",
        "3 attempts",
        "VELESDB_MEMORY_OLLAMA_URL",
        "VELESDB_MEMORY_OLLAMA_MODEL",
        "VELESDB_MEMORY_EMBEDDER=hash",
    ] {
        assert!(
            message.contains(needle),
            "the failure must name {needle:?} to be actionable, got: {message}"
        );
    }
    assert!(
        elapsed < Duration::from_secs(15),
        "retrying must not turn a fast failure into a long wait, took {elapsed:?}"
    );
    drop(handle);
}

#[test]
fn the_shipped_timeout_stays_bounded_and_usable() {
    // Low enough that an MCP client is still waiting when it fires,
    // high enough to survive a cold model load.
    assert!((5..=120).contains(&EMBED_TIMEOUT_SECS));
}

#[test]
#[ignore = "requires a local Ollama with an embedding model (ollama pull all-minilm)"]
fn embeds_through_a_running_ollama() {
    let embedder =
        OllamaEmbedder::new(DEFAULT_OLLAMA_URL, DEFAULT_OLLAMA_MODEL).expect("connect to ollama");
    let vector = embedder
        .embed("parking_lot avoids lock poisoning")
        .expect("embed");
    assert_eq!(vector.len(), embedder.dimension());
    assert!(vector
        .iter()
        .any(|&component| component.abs() > f32::EPSILON));
}
