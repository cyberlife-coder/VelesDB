//! Per-request observability for the daemon (#1780).
//!
//! The daemon used to emit NOTHING per request — `#1727` was diagnosed twice
//! on a wrong cause because no trace could answer the most elementary
//! incident question: "did the request reach the daemon at all?". These
//! tests pin the contract that closes that gap:
//!
//! - a handled tool call leaves one event carrying the tool name, a verdict
//!   and a duration (T1);
//! - a request on an unknown session leaves a transport event with its 404,
//!   distinguishable from a handled request's 2xx (T2) — so the three
//!   outside-identical cases (never arrived / refused / handled) are told
//!   apart by the log alone;
//! - no event ever carries fact content (T4) — the issue's privacy line:
//!   a session id and a tool name suffice;
//! - nor does any ERROR-path event carry client input (T5) — the leak the
//!   #1780 review proved in execution before this preset pinned it out;
//! - the idle retirement of a session worker — THE #1727 signal — is
//!   actually visible under the preset (T6).
//!
//! T3 — the silence-by-default and invalid-filter-refusal contract — is a
//! unit test in `src/logging.rs`, not here: it is about installing NO
//! subscriber, which cannot be observed under this file's process-global
//! capturing one.
//!
//! Capture mechanics: `tracing` events only reach a subscriber, and a
//! process has ONE global subscriber slot — so a single capturing
//! subscriber is installed once for the whole test binary, and the tests
//! serialize on a lock and clear the buffer between runs. Thread-local
//! (`with_default`) capture would silently MISS events from tasks the
//! server spawns onto other worker threads, which is exactly where rmcp
//! runs session workers — a capture that can't see the events it audits
//! would pass vacuously.
//!
//! The capture filters with [`velesdb_memory::logging::INCIDENT_PRESET`]
//! — the exact preset the installer deploys — NOT a blanket `trace`. That is
//! the whole point of T4 and T5: rmcp itself dumps full request arguments
//! (fact text included) at `rmcp::service=debug`, quotes client input in its
//! `warn`-level `response error` event on the same target, and dumps whole
//! messages at transport `trace` — so "no payload in the log" is only true
//! OF A FILTER, and the filter these tests prove is the one operators
//! actually run.

use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, DEFAULT_DIMENSION};

// --- Capturing subscriber ---------------------------------------------------

/// Shared byte buffer the capturing subscriber's writer appends to.
#[derive(Clone, Default)]
struct Capture {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl Capture {
    fn clear(&self) {
        self.buffer.lock().expect("capture lock").clear();
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.buffer.lock().expect("capture lock")).into_owned()
    }
}

/// One writer handle per fmt call — appends under the shared lock.
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("capture lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = CaptureWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CaptureWriter(Arc::clone(&self.buffer))
    }
}

/// Install the process-wide capturing subscriber (once) and serialize the
/// calling test until its `MutexGuard` drops. Every test MUST hold the guard
/// for its whole body: the buffer is process-global, so two tests running
/// concurrently would read each other's events.
fn capture_for_test() -> (Capture, MutexGuard<'static, ()>) {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    static SEQ: Mutex<()> = Mutex::new(());

    // A failed test panics while holding the guard, poisoning it; the NEXT
    // test must still fail on its own assertion, not on the poison — three
    // real failures beat one real and two `PoisonError` echoes.
    let guard = SEQ
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let capture = CAPTURE
        .get_or_init(|| {
            let capture = Capture::default();
            let subscriber = tracing_subscriber::fmt()
                .with_ansi(false)
                .with_env_filter(tracing_subscriber::EnvFilter::new(
                    velesdb_memory::logging::INCIDENT_PRESET,
                ))
                .with_writer(capture.clone())
                .finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("install the capturing subscriber once per process");
            capture
        })
        .clone();
    capture.clear();
    (capture, guard)
}

// --- Server + client harness (same shape as tests/http_transport.rs) --------

struct TestServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
    ct: CancellationToken,
    _store_dir: tempfile::TempDir,
}

async fn spawn_server() -> TestServer {
    spawn_server_with_keep_alive(velesdb_memory::http::DEFAULT_HTTP_KEEP_ALIVE).await
}

/// [`spawn_server`], with the session idle timeout injected — T6 retires a
/// session in milliseconds instead of the production hour.
async fn spawn_server_with_keep_alive(keep_alive: std::time::Duration) -> TestServer {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(DEFAULT_DIMENSION));
    let service =
        MemoryService::open(store_dir.path(), embedder).expect("open scratch memory store");
    let server = McpServer::new(service);

    let ct = CancellationToken::new();
    let app = velesdb_memory::http::router_with_limits_and_keep_alive(
        server,
        ct.child_token(),
        velesdb_memory::http::DEFAULT_HTTP_MAX_BODY_BYTES,
        velesdb_memory::http::DEFAULT_HTTP_MAX_SESSIONS,
        Some(keep_alive),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral loopback port");
    let addr = listener.local_addr().expect("read bound local addr");

    let shutdown_ct = ct.clone();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown_ct.cancelled_owned().await })
            .await;
    });

    TestServer {
        addr,
        handle,
        ct,
        _store_dir: store_dir,
    }
}

async fn shutdown(server: TestServer) {
    server.ct.cancel();
    server
        .handle
        .await
        .expect("http server task must not panic");
}

async fn connect(addr: SocketAddr) -> RunningService<RoleClient, ClientInfo> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp")),
    );
    ClientInfo::default()
        .serve(transport)
        .await
        .expect("MCP initialize handshake over HTTP")
}

fn as_args(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("expected a JSON object, got {other:?}"),
    }
}

// --- T1: a handled tool call is traceable ------------------------------------

#[test]
fn a_handled_tool_call_leaves_tool_verdict_and_duration() {
    let (capture, _seq) = capture_for_test();
    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    rt.block_on(async {
        let server = spawn_server().await;
        let client = connect(server.addr).await;
        client
            .call_tool(
                CallToolRequestParams::new("recall")
                    .with_arguments(as_args(json!({ "query": "anything", "limit": 3 }))),
            )
            .await
            .expect("recall call over HTTP");
        client.cancel().await.expect("close the MCP client");
        shutdown(server).await;
    });

    let log = capture.text();
    assert!(
        log.contains("tool=recall"),
        "a handled call must leave an event naming the tool — got:\n{log}"
    );
    assert!(
        log.contains("verdict=ok"),
        "the event must carry the call's verdict — got:\n{log}"
    );
    assert!(
        log.contains("elapsed_ms="),
        "the event must carry the call's duration — got:\n{log}"
    );
}

// --- T2: a refused request is distinguishable from a handled one -------------

#[test]
fn an_unknown_session_leaves_a_404_transport_event() {
    let (capture, _seq) = capture_for_test();
    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    let (status, log) = rt.block_on(async {
        let server = spawn_server().await;

        // Positive control FIRST: a real handshake must leave 2xx transport
        // events — it proves this test can see transport events at all, so
        // the 404 assertion below cannot pass vacuously.
        let client = connect(server.addr).await;
        client.cancel().await.expect("close the MCP client");

        let response = reqwest::Client::new()
            .post(format!("http://{}/mcp", server.addr))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-session-id", "no-such-session")
            .body(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": { "name": "recall", "arguments": { "query": "x" } }
                })
                .to_string(),
            )
            .send()
            .await
            .expect("POST to /mcp with an unknown session id");
        let status = response.status().as_u16();
        shutdown(server).await;
        (status, capture.text())
    });

    assert_eq!(
        status, 404,
        "an unknown session id must be refused with 404"
    );
    assert!(
        log.contains("status=200") || log.contains("status=202"),
        "the handshake's transport events are the positive control — got:\n{log}"
    );
    assert!(
        log.contains("status=404"),
        "the refusal must leave a transport event carrying its 404 — got:\n{log}"
    );
    assert!(
        log.contains("session=no-such-session"),
        "the refused event must name the session that was refused — got:\n{log}"
    );
}

// --- T4: no event ever carries fact content ----------------------------------

#[test]
fn events_never_carry_fact_content() {
    const CANARY: &str = "CANARY-9f3a1c-le-contenu-ne-doit-jamais-fuiter";

    let (capture, _seq) = capture_for_test();
    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    rt.block_on(async {
        let server = spawn_server().await;
        let client = connect(server.addr).await;
        client
            .call_tool(
                CallToolRequestParams::new("remember")
                    .with_arguments(as_args(json!({ "fact": CANARY }))),
            )
            .await
            .expect("remember call over HTTP");
        client.cancel().await.expect("close the MCP client");
        shutdown(server).await;
    });

    let log = capture.text();
    // Positive half first: the capture DID see this very call — without it,
    // an empty capture would "prove" privacy vacuously.
    assert!(
        log.contains("tool=remember"),
        "the call must be traced (positive control for the canary check) — got:\n{log}"
    );
    assert!(
        !log.contains(CANARY),
        "an event carried fact content — the issue's privacy line forbids \
         payloads in traces:\n{log}"
    );
}

// --- T5: the ERROR path never carries client content either ------------------

#[test]
fn error_responses_never_carry_client_content() {
    // T4 proves the happy path; this is the leak adversarial review caught.
    // A failing tool body propagates as `Err(ErrorData)`, and rmcp's service
    // logs `response error` with the whole `ErrorData` — message included —
    // at WARN under `rmcp::service`. Several `MemoryError` messages quote
    // client input verbatim: `recall_where`'s column filters embed the
    // offending FIELD name on an invalid identifier, and the full offending
    // VALUE on a non-scalar (`validate_column_filter`, storage.rs). The
    // incident preset must keep that WARN out of the log.
    const FIELD_CANARY: &str = "CANARY-FIELD-7d2b1e not a valid identifier";
    const VALUE_CANARY: &str = "CANARY-VALUE-4c9a0f-le-contenu-du-client";

    let (capture, _seq) = capture_for_test();
    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    rt.block_on(async {
        let server = spawn_server().await;
        let client = connect(server.addr).await;
        // Two REFUSED calls, each canary travelling in its own error: an
        // invalid filter field (the message quotes the field name), then a
        // non-scalar filter value (the message quotes the whole value).
        for entry in [
            json!({ "field": FIELD_CANARY, "op": "eq", "value": "x" }),
            json!({ "field": "okfield", "op": "eq", "value": { "secret": VALUE_CANARY } }),
        ] {
            let refused = client
                .call_tool(
                    CallToolRequestParams::new("recall_where").with_arguments(as_args(
                        json!({ "query": "anything", "filters": [entry] }),
                    )),
                )
                .await;
            assert!(
                refused.is_err() || refused.is_ok_and(|r| r.is_error == Some(true)),
                "the invalid filter must be refused — a filter silently \
                 accepted means this test stopped exercising the error path"
            );
        }
        client.cancel().await.expect("close the MCP client");
        shutdown(server).await;
    });

    let log = capture.text();
    // Positive control: the refused calls were traced with an error verdict —
    // an empty capture would prove nothing.
    assert!(
        log.contains("tool=recall_where") && log.contains("verdict=error"),
        "the refused calls must be traced with their verdict (positive \
         control) — got:\n{log}"
    );
    assert!(
        !log.contains("CANARY-FIELD") && !log.contains("CANARY-VALUE"),
        "an error event carried client content under the incident preset — \
         the preset must exclude rmcp's response-error dump:\n{log}"
    );
}

// --- T6: the preset actually captures THE #1727 signal -----------------------

#[test]
fn an_idle_session_leaves_the_worker_quit_signal() {
    // The preset's `rmcp::transport::worker=debug` directive exists for one
    // event: a session worker dying of inactivity while the session stays in
    // the table — the #1727 mechanism. A preset that promised that signal
    // and filtered it out would be exactly the kind of claim nothing
    // re-verifies, so this retires a real session (a keep-alive of
    // milliseconds instead of the production hour) and reads the signal back.
    let (capture, _seq) = capture_for_test();
    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    rt.block_on(async {
        let server = spawn_server_with_keep_alive(std::time::Duration::from_millis(150)).await;
        let client = connect(server.addr).await;
        // Sit idle past the keep-alive and POLL for the signal instead of
        // sleeping a fixed once: a loaded runner can stretch the retirement
        // well past its nominal deadline, and a fixed sleep is how this
        // transport's tests went flaky before (#1793). The client stays
        // alive throughout — closing it first would end the session for the
        // wrong reason and prove nothing about idleness.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let log = capture.text();
            if log.contains("worker quit with reason") && log.contains("IdleTimeout") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the idle retirement must be visible under the incident \
                 preset within 10s — got:\n{log}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        drop(client);
        shutdown(server).await;
    });
}
