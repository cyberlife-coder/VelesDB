//! HTTP (streamable) transport for the MCP server — multi-client mode.
//!
//! `velesdb-memory` today only speaks stdio: every MCP client (Claude Code,
//! Claude Desktop, Windsurf, …) spawns its own server process, and the
//! store's single-writer `flock` means only one of those processes can
//! actually hold the store open at a time — so only one client can use
//! memory at once. The fix is a single HTTP daemon multiple clients share.
//!
//! These tests build the axum [`Router`](axum::Router) directly via
//! `velesdb_memory::http::router` (no subprocess) and drive it with a real
//! MCP client over the streamable-HTTP transport (`rmcp`'s own client-side
//! transport, the same one exercised by rmcp's upstream test suite), bound
//! to an OS-assigned loopback port so tests never collide on a fixed one.
//!
//! The concurrency test is the risk this transport exists to retire:
//! `Database`'s internal `RwLock` (velesdb-core) makes concurrent requests
//! against the ONE shared store safe in-process — the `flock` only ever
//! guarded cross-*process* access, which HTTP sidesteps entirely by having
//! exactly one process own the store. Twenty simultaneous `remember`s (and a
//! `remember`+`recall` mix) must all complete with no panic and no deadlock.

use std::net::SocketAddr;

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

/// A running HTTP transport for a test: the bound address, the server task
/// (drive it to completion via [`shutdown`]), the token that stops it, and
/// the store's `TempDir` (kept alive for the test's duration — dropping it
/// early would delete the store out from under the server).
struct TestServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
    ct: CancellationToken,
    _store_dir: tempfile::TempDir,
}

/// Cancel the server's token and wait for its task to actually finish —
/// every test must call this before returning so a failed/hung shutdown
/// surfaces as a test failure instead of a silently leaked task.
async fn shutdown(server: TestServer) {
    server.ct.cancel();
    server
        .handle
        .await
        .expect("http server task must not panic");
}

/// Spin up the HTTP transport on `127.0.0.1:0` (OS-assigned port) backed by
/// a fresh scratch store — the same `HashEmbedder` + `MemoryService::open`
/// setup `src/mcp/server_tests.rs` uses for the stdio-side unit tests, just
/// wrapped in the new HTTP router instead of called directly.
async fn spawn_http_server() -> TestServer {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(DEFAULT_DIMENSION));
    let service =
        MemoryService::open(store_dir.path(), embedder).expect("open scratch memory store");
    let server = McpServer::new(service);

    let ct = CancellationToken::new();
    let app = velesdb_memory::http::router(server, ct.child_token());
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

/// Complete the MCP `initialize` handshake against `addr`'s `/mcp` endpoint
/// and return the connected client. `ServiceExt::serve` performs
/// `initialize` as part of establishing the session, so a successful
/// `connect` IS the initialize round trip.
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

/// Call `remember` over HTTP and return the fact's `id_str`.
async fn remember(client: &RunningService<RoleClient, ClientInfo>, fact: &str) -> String {
    let result = client
        .call_tool(
            CallToolRequestParams::new("remember").with_arguments(as_args(json!({ "fact": fact }))),
        )
        .await
        .expect("remember call over HTTP");
    let structured = result
        .structured_content
        .expect("remember returns structured_content");
    structured["id_str"]
        .as_str()
        .expect("id_str is a string")
        .to_owned()
}

/// Call `recall` over HTTP and return whether any hit's `content` exactly
/// matches `needle`.
async fn recall_contains(
    client: &RunningService<RoleClient, ClientInfo>,
    query: &str,
    needle: &str,
) -> bool {
    let result = client
        .call_tool(
            CallToolRequestParams::new("recall").with_arguments(as_args(json!({
                "query": query,
                "limit": 50,
            }))),
        )
        .await
        .expect("recall call over HTTP");
    let structured = result
        .structured_content
        .expect("recall returns structured_content");
    let memories = structured["memories"]
        .as_array()
        .expect("memories is an array");
    memories
        .iter()
        .any(|memory| memory["content"].as_str() == Some(needle))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn initialize_round_trip_succeeds_over_http() {
    let server = spawn_http_server().await;

    let client = connect(server.addr).await;
    let info = client
        .peer_info()
        .expect("server must advertise its info during initialize");
    assert_eq!(info.server_info.name, "velesdb-memory");

    shutdown(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn remember_then_recall_roundtrip_over_http() {
    let server = spawn_http_server().await;
    let client = connect(server.addr).await;

    let fact = "HTTP transport lets many MCP clients share one memory daemon";
    let id_str = remember(&client, fact).await;
    assert!(!id_str.is_empty(), "remember must return a non-empty id");

    let found = recall_contains(&client, "HTTP transport memory daemon", fact).await;
    assert!(found, "the remembered fact must be recallable over HTTP");

    shutdown(server).await;
}

/// The central risk this transport exists to retire: 20 simultaneous
/// `remember` calls against the ONE shared store — each from its OWN
/// connected client, exactly like 20 real MCP clients (Claude Code, Claude
/// Desktop, Windsurf, …) sharing one daemon rather than each spawning its
/// own stdio process — must all succeed with unique ids and never panic.
/// This proves `Database`'s internal locking (velesdb-core) is enough on its
/// own without the process-level `flock`, which never applies here: HTTP
/// concurrency is many *sessions* in ONE process, not many processes.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn twenty_concurrent_remembers_all_succeed_with_unique_ids() {
    let server = spawn_http_server().await;

    let mut tasks = Vec::with_capacity(20);
    for i in 0..20 {
        let addr = server.addr;
        tasks.push(tokio::spawn(async move {
            let client = connect(addr).await;
            remember(&client, &format!("concurrent fact number {i}")).await
        }));
    }

    let mut ids = std::collections::HashSet::new();
    for task in tasks {
        let id = task.await.expect("remember task must not panic");
        assert!(ids.insert(id), "remember must never return a duplicate id");
    }
    assert_eq!(ids.len(), 20, "all 20 concurrent remembers must succeed");

    shutdown(server).await;
}

/// A mixed `remember` + `recall` race (again, one connection per task —
/// many concurrent clients, not multiplexed calls on one session): proves
/// the HTTP transport has no deadlock or corruption when reads and writes
/// overlap, then — after the race settles — that every fact written during
/// it is actually recallable (not silently dropped or corrupted).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_remember_and_recall_do_not_deadlock_and_all_facts_recallable() {
    let server = spawn_http_server().await;

    let seed_client = connect(server.addr).await;
    remember(&seed_client, "seed fact alpha for the concurrency race").await;
    remember(&seed_client, "seed fact beta for the concurrency race").await;
    drop(seed_client);

    let mut remember_tasks = Vec::with_capacity(10);
    for i in 0..10 {
        let addr = server.addr;
        remember_tasks.push(tokio::spawn(async move {
            let client = connect(addr).await;
            remember(&client, &format!("racing fact {i}")).await
        }));
    }

    let mut recall_tasks = Vec::with_capacity(10);
    for _ in 0..10 {
        let addr = server.addr;
        recall_tasks.push(tokio::spawn(async move {
            let client = connect(addr).await;
            // Never asserted mid-race: the race may see a fact before it is
            // durably stored. This only proves recall doesn't panic/hang
            // while writes are in flight.
            let _ = recall_contains(&client, "racing fact", "irrelevant").await;
        }));
    }

    for task in remember_tasks {
        task.await.expect("remember task must not panic");
    }
    for task in recall_tasks {
        task.await
            .expect("recall task must not panic during the race");
    }

    let verify_client = connect(server.addr).await;
    for i in 0..10 {
        let fact = format!("racing fact {i}");
        assert!(
            recall_contains(&verify_client, &fact, &fact).await,
            "fact {i} written during the concurrent race must be recallable afterwards"
        );
    }
    assert!(
        recall_contains(
            &verify_client,
            "seed fact",
            "seed fact alpha for the concurrency race"
        )
        .await
    );
    drop(verify_client);

    shutdown(server).await;
}
