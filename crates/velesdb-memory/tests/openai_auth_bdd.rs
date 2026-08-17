//! BDD integration tests for #1751: what the OpenAI-compatible backends
//! actually put on the wire when authenticating.
//!
//! **These assert on BYTES, not on an in-process request object.** A mock that
//! inspects a builder proves what the code meant to send; only reading the
//! socket proves what it sent. The rule under test has a negative half — "no
//! `Authorization` header AT ALL when no token is configured" — and a negative
//! is exactly what a builder-level assertion is worst at: it cannot distinguish
//! "never set" from "set and dropped in transit".
//!
//! Both roles are covered, not the shared client twice. The client is
//! role-agnostic by construction, but each adapter builds its own request, and
//! an adapter is free to add or lose a header on the way. Testing only the
//! client would leave that gap open on both sides.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

#![cfg(all(feature = "embedder-http", feature = "extractor-http"))]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener};
use std::thread::JoinHandle;

use velesdb_memory::extract::Extractor as _;
use velesdb_memory::{Auth, OpenAiEmbedder, OpenAiExtractor};

/// A well-formed `OpenAI` embeddings response — two dimensions is enough for the
/// constructor's dimension probe to succeed.
const EMBEDDINGS_RESPONSE: &str = r#"{"data":[{"embedding":[0.5,0.5]}]}"#;

/// A well-formed `OpenAI` chat-completions response whose `content` is the strict
/// JSON array the extraction prompt asks the model for. Empty on purpose: this
/// suite is about headers, not about what was extracted.
const CHAT_RESPONSE: &str = r#"{"choices":[{"message":{"content":"[]"}}]}"#;

/// Accept exactly one connection, capture the raw request bytes, answer
/// `body`, and hand the captured text back through the join handle.
///
/// Bounded to a single connection so the thread ends on its own — no join
/// timeout, no orphan left behind if an assertion panics first.
fn capture_one_request(body: &'static str) -> (SocketAddr, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let Ok((mut socket, _)) = listener.accept() else {
            return String::new();
        };
        // Read the WHOLE request before answering. A single read is a race:
        // the extraction prompt is several kilobytes, so the request arrives
        // in more than one chunk, and answering after the first one closes the
        // socket while the client is still writing — which surfaces as an
        // opaque transport error instead of the assertion under test.
        let captured = read_full_request(&mut socket);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes());
        let _ = socket.flush();
        captured
    });
    (addr, handle)
}

/// Read one complete HTTP request: headers, then exactly `Content-Length`
/// bytes of body.
///
/// Bounded on both ends — a total ceiling and a read timeout — so a client
/// that stops mid-request ends this thread instead of wedging the suite.
fn read_full_request(socket: &mut std::net::TcpStream) -> String {
    const CEILING: usize = 256 * 1024;
    let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let Ok(read) = socket.read(&mut chunk) else {
            break;
        };
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        if raw.len() >= CEILING {
            break;
        }
        let text = String::from_utf8_lossy(&raw);
        let Some(head_end) = text.find("\r\n\r\n") else {
            continue; // headers still incomplete
        };
        let declared = content_length(&text[..head_end]);
        if raw.len() >= head_end + 4 + declared {
            break;
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}

/// The `Content-Length` a header block declares, or `0` when it declares none.
fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0)
}

/// The header lines of a captured request, lowercased for a case-insensitive
/// search — HTTP header names are case-insensitive, so a test that only looked
/// for `Authorization` could be fooled by `authorization`.
fn header_block(captured: &str) -> String {
    captured
        .split("\r\n\r\n")
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

// --- Nominal: no token configured -------------------------------------------

#[test]
fn embedder_sends_no_authorization_header_when_no_token_is_configured() {
    let (addr, server) = capture_one_request(EMBEDDINGS_RESPONSE);

    let embedder = OpenAiEmbedder::new(
        format!("http://{addr}"),
        "text-embedding-3-small",
        Auth::None,
    );
    let captured = server.join().expect("server thread");
    embedder.expect("the probe response is well-formed");

    let headers = header_block(&captured);
    assert!(
        !headers.contains("authorization"),
        "a local server needs no credential: with no token configured the \
         request must carry NO `Authorization` header at all, got:\n{captured}"
    );
}

#[test]
fn extractor_sends_no_authorization_header_when_no_token_is_configured() {
    let (addr, server) = capture_one_request(CHAT_RESPONSE);

    let extractor = OpenAiExtractor::new(format!("http://{addr}"), "qwen3", Auth::None);
    extractor
        .extract("Alice ships the parser.")
        .expect("extract");
    let captured = server.join().expect("server thread");

    let headers = header_block(&captured);
    assert!(
        !headers.contains("authorization"),
        "the rule is shared with the embedder, and must hold on this role too, \
         got:\n{captured}"
    );
}

// --- Nominal: a token IS configured -----------------------------------------

#[test]
fn embedder_sends_the_exact_bearer_header_when_a_token_is_configured() {
    let (addr, server) = capture_one_request(EMBEDDINGS_RESPONSE);

    let embedder = OpenAiEmbedder::new(
        format!("http://{addr}"),
        "text-embedding-3-small",
        Auth::Bearer("sk-test-123".to_owned()),
    );
    let captured = server.join().expect("server thread");
    embedder.expect("the probe response is well-formed");

    assert!(
        captured.contains("Authorization: Bearer sk-test-123"),
        "the header must be exactly `Authorization: Bearer <token>`, got:\n{captured}"
    );
}

#[test]
fn extractor_sends_the_exact_bearer_header_when_a_token_is_configured() {
    let (addr, server) = capture_one_request(CHAT_RESPONSE);

    let extractor = OpenAiExtractor::new(
        format!("http://{addr}"),
        "qwen3",
        Auth::Bearer("sk-test-123".to_owned()),
    );
    extractor
        .extract("Alice ships the parser.")
        .expect("extract");
    let captured = server.join().expect("server thread");

    assert!(
        captured.contains("Authorization: Bearer sk-test-123"),
        "the header must be exactly `Authorization: Bearer <token>`, got:\n{captured}"
    );
}

// --- Edge: a provider that authenticates by another header ------------------

#[test]
fn a_custom_auth_header_is_sent_verbatim_and_no_bearer_is_added() {
    // Azure OpenAI authenticates with `api-key`, not `Authorization`. This is
    // why `Auth` is an enum rather than an `Option<String>` token: the shape
    // was already known to vary before the first non-OpenAI provider landed.
    let (addr, server) = capture_one_request(EMBEDDINGS_RESPONSE);

    let embedder = OpenAiEmbedder::new(
        format!("http://{addr}"),
        "text-embedding-3-small",
        Auth::Header {
            name: "api-key".to_owned(),
            value: "azure-secret".to_owned(),
        },
    );
    let captured = server.join().expect("server thread");
    embedder.expect("the probe response is well-formed");

    assert!(
        captured.contains("api-key: azure-secret"),
        "the caller's header must go out verbatim, got:\n{captured}"
    );
    assert!(
        !header_block(&captured).contains("authorization"),
        "naming another header must not ALSO add a bearer, got:\n{captured}"
    );
}

// --- Negative: the secret must not leak through Debug -----------------------

#[test]
fn debug_never_prints_the_secret() {
    // A token reaches logs and panic messages through `Debug` far more often
    // than through a deliberate print. The same reflex the crate already
    // applies to `ExtractorSelection`, which refuses to dump a backend's URL.
    let bearer = format!("{:?}", Auth::Bearer("sk-must-not-appear".to_owned()));
    assert!(
        !bearer.contains("sk-must-not-appear"),
        "a bearer token must be redacted in Debug, got: {bearer}"
    );

    let custom = format!(
        "{:?}",
        Auth::Header {
            name: "api-key".to_owned(),
            value: "azure-must-not-appear".to_owned(),
        }
    );
    assert!(
        !custom.contains("azure-must-not-appear"),
        "a custom header VALUE must be redacted in Debug, got: {custom}"
    );
    assert!(
        custom.contains("api-key"),
        "the header NAME is not a secret and stays visible, so a misconfigured \
         provider is still diagnosable: {custom}"
    );
}

// --- The request LINE, not just its headers ---------------------------------

#[test]
fn a_base_url_copied_with_its_version_prefix_still_hits_the_right_path() {
    // Byte-level, for the same reason the header tests are: only the socket
    // shows whether the path was doubled. The URL under test is the literal
    // string an oMLX console offers to copy — `http://127.0.0.1:8019/v1` —
    // which a naive concatenation turns into `/v1/v1/chat/completions`.
    let (addr, server) = capture_one_request(CHAT_RESPONSE);
    let extractor = OpenAiExtractor::new(format!("http://{addr}/v1"), "ornith-35b", Auth::None);
    let _ = extractor.extract("peu importe le texte");
    let captured = server.join().expect("stub thread");
    let request_line = captured.lines().next().unwrap_or_default();

    assert!(
        request_line.contains("/v1/chat/completions"),
        "the protocol path must be reached exactly once, got: {request_line}"
    );
    assert!(
        !request_line.contains("/v1/v1/"),
        "a base URL that already carries the version prefix must not double it \
         — that is a 404 whose cause is invisible. Got: {request_line}"
    );
}

#[test]
fn an_origin_without_the_prefix_reaches_the_same_path() {
    // The control: both spellings must land on one identical request line, or
    // the normalisation above traded one surprise for another.
    let (addr, server) = capture_one_request(CHAT_RESPONSE);
    let extractor = OpenAiExtractor::new(format!("http://{addr}"), "ornith-35b", Auth::None);
    let _ = extractor.extract("peu importe le texte");
    let captured = server.join().expect("stub thread");
    assert!(
        captured
            .lines()
            .next()
            .unwrap_or_default()
            .contains("/v1/chat/completions"),
        "got: {captured}"
    );
}
