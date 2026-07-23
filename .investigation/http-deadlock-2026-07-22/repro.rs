//! ADVERSARIAL REVIEW SCRATCH TEST — NOT PART OF THE PR.
//!
//! The PR's concurrency tests use one HTTP connection per task. Real MCP
//! clients keep ONE stateful session open and multiplex many tool calls on
//! it. This drives N concurrent `remember` calls through a SINGLE shared MCP
//! session (cloned `Peer`) on a multi-thread runtime (matching the real
//! daemon). Phased short timeouts + progress markers isolate WHERE it hangs,
//! and the process self-terminates (exit 101 after a grace window for stack
//! sampling) so it can never wedge the harness.

use std::net::SocketAddr;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, DEFAULT_DIMENSION};

fn as_args(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("expected a JSON object, got {other:?}"),
    }
}

async fn spawn_http_server() -> (SocketAddr, CancellationToken, tempfile::TempDir) {
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
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown_ct.cancelled_owned().await })
            .await;
    });

    (addr, ct, store_dir)
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

fn hang_and_die(marker: &str) -> ! {
    eprintln!("MARKER_HANG_DETECTED phase={marker} (grace window for `sample`, then exit 101)");
    std::thread::sleep(Duration::from_secs(20));
    std::process::exit(101);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn single_shared_session_survives_concurrent_multiplexed_calls() {
    let (addr, _ct, _store) = spawn_http_server().await;

    let client = match tokio::time::timeout(Duration::from_secs(10), connect(addr)).await {
        Ok(client) => client,
        Err(_) => hang_and_die("connect"),
    };
    eprintln!("MARKER_CONNECTED");
    let peer = client.peer().clone();

    // Sanity: one sequential call on the shared session (the PR's own tests
    // prove this pattern works; a hang here would be something else).
    let sequential = peer.call_tool(
        CallToolRequestParams::new("remember")
            .with_arguments(as_args(json!({ "fact": "sequential warmup fact" }))),
    );
    if tokio::time::timeout(Duration::from_secs(10), sequential)
        .await
        .is_err()
    {
        hang_and_die("sequential_single_call");
    }
    eprintln!("MARKER_SEQUENTIAL_OK");

    // The real probe: only FOUR concurrent calls multiplexed on the ONE session.
    let mut tasks = Vec::new();
    for i in 0..4 {
        let peer = peer.clone();
        tasks.push(tokio::spawn(async move {
            peer.call_tool(
                CallToolRequestParams::new("remember")
                    .with_arguments(as_args(json!({ "fact": format!("shared fact {i}") }))),
            )
            .await
            .expect("remember over the SHARED session")
        }));
    }
    let all = async {
        for task in tasks {
            task.await.expect("task must not panic");
        }
    };
    if tokio::time::timeout(Duration::from_secs(15), all)
        .await
        .is_err()
    {
        hang_and_die("four_concurrent_calls_shared_session");
    }
    eprintln!("MARKER_CONCURRENT_OK");

    // Scale up: 20 concurrent remembers + 10 concurrent recalls on the SAME
    // shared session — the load shape that hung for 5+ minutes on the first
    // reproduction attempt.
    let mut tasks = Vec::new();
    for i in 0..20 {
        let peer = peer.clone();
        tasks.push(tokio::spawn(async move {
            peer.call_tool(
                CallToolRequestParams::new("remember")
                    .with_arguments(as_args(json!({ "fact": format!("storm fact {i}") }))),
            )
            .await
            .expect("remember over the SHARED session (storm)")
        }));
    }
    for _ in 0..10 {
        let peer = peer.clone();
        tasks.push(tokio::spawn(async move {
            peer.call_tool(
                CallToolRequestParams::new("recall")
                    .with_arguments(as_args(json!({ "query": "storm fact", "limit": 50 }))),
            )
            .await
            .expect("recall over the SHARED session (storm)")
        }));
    }
    let all = async {
        for task in tasks {
            task.await.expect("storm task must not panic");
        }
    };
    if tokio::time::timeout(Duration::from_secs(20), all)
        .await
        .is_err()
    {
        hang_and_die("thirty_concurrent_calls_shared_session");
    }
    eprintln!("MARKER_STORM_OK");
}
