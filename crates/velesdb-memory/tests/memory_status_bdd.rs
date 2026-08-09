//! Behaviour: `memory_status` answers, INSIDE the protocol, the questions a
//! user otherwise discovers only through degraded recall — which embedder is
//! actually running, whether recall is semantic or lexical, whether
//! extraction is wired, and whether the graph has any edges at all.
//!
//! The audit finding this closes: the hash-embedder warning goes to stderr
//! of a stdio server, and every mainstream MCP client swallows that stream.
//! A user on the default build experiences "recall is bad", never sees why,
//! and cannot ask. The agent can — if the server exposes the answer as a
//! tool. Same story for a flat graph: `why()` silently degrades to search
//! when nothing ever wired an edge, and only an edge count says so.
//!
//! Wire-level like the other BDD suites: the REAL server over an in-memory
//! duplex, raw tool calls, assertions on `structuredContent`.

// `mcp` (which implies `persistence`), not `persistence`: this file boots the
// MCP server itself, and the feature-isolation matrix checks `persistence`
// ALONE with --all-targets — under which `McpServer` does not exist.
#![cfg(feature = "mcp")]

use rmcp::model::CallToolRequestParams;
use rmcp::service::ServiceExt;
use serde_json::Value;
use tempfile::TempDir;
use velesdb_memory::{DynEmbedder, HashEmbedder, McpServer, MemoryService, DEFAULT_DIMENSION};

/// Boot the real server over a duplex, with the embedder identity the binary
/// would attach (`hash` at [`DEFAULT_DIMENSION`]), and hand back the client.
async fn connected() -> (
    TempDir,
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
) {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(DEFAULT_DIMENSION));
    let service =
        MemoryService::open(store_dir.path(), embedder).expect("open scratch memory store");
    let server = McpServer::new(service)
        .with_embedder_identity("hash", DEFAULT_DIMENSION)
        .with_store_dir(store_dir.path());
    let (server_side, client_side) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_side).await {
            let _ = running.waiting().await;
        }
    });
    let client = ().serve(client_side).await.expect("MCP initialize handshake over duplex");
    (store_dir, client)
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    tool: &str,
    args: Value,
) -> Value {
    let mut request = CallToolRequestParams::new(tool.to_owned());
    if let Value::Object(map) = args {
        request = request.with_arguments(map);
    }
    let result = client
        .call_tool(request)
        .await
        .unwrap_or_else(|e| panic!("tool {tool} failed: {e}"));
    result
        .structured_content
        .unwrap_or_else(|| panic!("tool {tool} returned no structuredContent"))
}

#[tokio::test]
async fn status_names_the_embedder_and_says_recall_is_not_semantic() {
    let (_dir, client) = connected().await;
    let status = call(&client, "memory_status", Value::Null).await;

    let embedder = &status["embedder"];
    assert_eq!(embedder["model"], "hash", "the running model is named");
    assert_eq!(
        embedder["dimension"],
        Value::from(DEFAULT_DIMENSION as u64),
        "the vector width is reported"
    );
    assert_eq!(
        embedder["semantic"], false,
        "hash must be reported as NOT semantic — this flag is the whole \
         point: it is what the stderr warning says into the void"
    );
}

#[tokio::test]
async fn status_reports_the_stores_provenance_record() {
    let (_dir, client) = connected().await;
    let status = call(&client, "memory_status", Value::Null).await;

    // `MemoryService::open` in this test writes no provenance record — only
    // the daemon's startup path does. The status must SAY it is unrecorded
    // rather than invent one: an absent record is a fact about the store.
    assert_eq!(
        status["provenance"]["recorded"], false,
        "no provenance was written, so none may be reported"
    );
}

#[tokio::test]
async fn status_counts_facts_and_edges_and_flags_the_flat_graph() {
    let (_dir, client) = connected().await;

    let before = call(&client, "memory_status", Value::Null).await;
    assert_eq!(before["memory"]["facts"], 0, "a fresh store holds no facts");
    assert_eq!(
        before["memory"]["edges"], 0,
        "a fresh store's graph has no edges"
    );

    let first = call(
        &client,
        "remember",
        serde_json::json!({"fact": "le port API est 6333 car 3000 etait pris"}),
    )
    .await;
    let second = call(
        &client,
        "remember",
        serde_json::json!({"fact": "le choix du port vient de l'incident INC-42"}),
    )
    .await;

    let stored = call(&client, "memory_status", Value::Null).await;
    assert_eq!(
        stored["memory"]["facts"], 2,
        "both remembered facts are counted"
    );
    assert_eq!(
        stored["memory"]["edges"], 0,
        "remember alone wires nothing — this zero is the observable 'why() \
         will behave like plain search' state the tool exists to surface"
    );

    call(
        &client,
        "relate",
        serde_json::json!({
            "from": first["id_str"],
            "to": second["id_str"],
            "relation": "explique"
        }),
    )
    .await;

    let wired = call(&client, "memory_status", Value::Null).await;
    let edges = wired["memory"]["edges"]
        .as_u64()
        .expect("edge count is a number");
    assert!(edges >= 1, "the wired edge shows up in the count");
}

#[tokio::test]
async fn status_says_whether_extraction_is_configured() {
    let (_dir, client) = connected().await;
    let status = call(&client, "memory_status", Value::Null).await;

    assert_eq!(
        status["extraction"]["configured"], false,
        "no extractor was attached, and the status must say so — \
         remember_extracted refusals become explainable"
    );
    assert_eq!(
        status["extraction"]["autograph_active"], false,
        "no autograph worker without an extractor"
    );
    assert_eq!(
        status["extraction"]["autograph_dropped"], 0,
        "the drop counter starts at zero and is relayed"
    );
}
